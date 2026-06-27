using System.Security.Cryptography;
using System.Text;
using System.Text.Json;
using System.Text.Json.Serialization;
using DocumentFormat.OpenXml;
using DocumentFormat.OpenXml.Packaging;
using DocumentFormat.OpenXml.Spreadsheet;
using DocumentFormat.OpenXml.Validation;
using DocumentFormat.OpenXml.Wordprocessing;

var exitCode = OfficeWorkerCli.Run(args);
Environment.Exit(exitCode);

internal static class OfficeWorkerCli
{
    private static readonly JsonSerializerOptions JsonOptions = new()
    {
        WriteIndented = true,
        DefaultIgnoreCondition = JsonIgnoreCondition.WhenWritingNull,
    };

    public static int Run(string[] args)
    {
        try
        {
            if (args.Length == 0 || args[0] is "--help" or "-h")
            {
                PrintHelp();
                return args.Length == 0 ? 2 : 0;
            }

            var command = args[0];
            var options = ParseOptions(args.Skip(1).ToArray());
            var receipt = command switch
            {
                "read-text" => ReadText(Required(options, "input")),
                "batch-read-text" => BatchReadText(Required(options, "input-list")),
                "read-workbook-text" => ReadWorkbook(
                    Required(options, "input"),
                    Optional(options, "sheet"),
                    OptionalInt(options, "max-rows") ?? 200,
                    "office.workbook.read_text"),
                "read-workbook-cells" => ReadWorkbook(
                    Required(options, "input"),
                    Optional(options, "sheet"),
                    OptionalInt(options, "max-rows") ?? 200,
                    "office.workbook.read_cells"),
                "create-docx" => CreateDocx(
                    Required(options, "output"),
                    TextInput(options),
                    Optional(options, "title")),
                "rewrite-save-as" => RewriteSaveAs(
                    Required(options, "input"),
                    Required(options, "output"),
                    TextInput(options)),
                "preview-in-place-rewrite" => PreviewInPlaceRewrite(Required(options, "input"), TextInput(options)),
                "rewrite-in-place" => RewriteInPlace(Required(options, "input"), TextInput(options)),
                "preview-rewrite" => PreviewRewrite(Required(options, "input"), TextInput(options)),
                "diff-summary" => DiffSummary(Required(options, "before"), Required(options, "after")),
                "validate" => ValidateDocx(Required(options, "input")),
                "self-test" => SelfTest(),
                _ => throw new ArgumentException($"unknown command: {command}"),
            };

            Emit(receipt, Optional(options, "receipt"));
            return receipt.Status == "success" ? 0 : 1;
        }
        catch (Exception ex)
        {
            var receipt = new OfficeReceipt(
                CapabilityId: "office.worker",
                Status: "failed",
                Data: new Dictionary<string, object?>
                {
                    ["error_type"] = ex.GetType().Name,
                    ["error"] = ex.Message,
                });
            Emit(receipt, null);
            return 1;
        }
    }

    private static OfficeReceipt ReadText(string inputPath)
    {
        using var document = WordprocessingDocument.Open(inputPath, false);
        var extraction = ExtractDocx(document);
        var validation = ValidateOpenXml(document);
        return new OfficeReceipt(
            "office.docx.read_text",
            validation.Count == 0 ? "success" : "failed",
            new Dictionary<string, object?>
            {
                ["source_path"] = Path.GetFullPath(inputPath),
                ["source_hash"] = Sha256File(inputPath),
                ["paragraphs"] = extraction.Paragraphs,
                ["headings"] = extraction.Headings,
                ["tables"] = extraction.Tables,
                ["style_ids"] = extraction.StyleIds.OrderBy(static item => item).ToArray(),
                ["text"] = string.Join(Environment.NewLine, extraction.Paragraphs),
                ["validation_errors"] = validation,
            });
    }

    private static OfficeReceipt BatchReadText(string inputListPath)
    {
        var documents = new List<Dictionary<string, object?>>();
        var errors = new List<Dictionary<string, object?>>();
        var inputPaths = File.ReadAllLines(inputListPath, Encoding.UTF8)
            .Select(static line => line.Trim())
            .Where(static line => !string.IsNullOrWhiteSpace(line))
            .Distinct(StringComparer.OrdinalIgnoreCase)
            .ToList();

        foreach (var inputPath in inputPaths)
        {
            try
            {
                using var document = WordprocessingDocument.Open(inputPath, false);
                var extraction = ExtractDocx(document);
                var validation = ValidateOpenXml(document);
                documents.Add(new Dictionary<string, object?>
                {
                    ["source_path"] = Path.GetFullPath(inputPath),
                    ["source_hash"] = Sha256File(inputPath),
                    ["paragraphs"] = extraction.Paragraphs,
                    ["headings"] = extraction.Headings,
                    ["tables"] = extraction.Tables,
                    ["style_ids"] = extraction.StyleIds.OrderBy(static item => item).ToArray(),
                    ["text"] = string.Join(Environment.NewLine, extraction.Paragraphs),
                    ["char_count"] = extraction.Paragraphs.Sum(static item => item.Length),
                    ["paragraph_count"] = extraction.Paragraphs.Count,
                    ["validation_errors"] = validation,
                });
            }
            catch (Exception ex)
            {
                errors.Add(new Dictionary<string, object?>
                {
                    ["source_path"] = Path.GetFullPath(inputPath),
                    ["error_type"] = ex.GetType().Name,
                    ["error"] = ex.Message,
                });
            }
        }

        return new OfficeReceipt(
            "office.docx.batch_read_text",
            errors.Count == 0 ? "success" : "partial",
            new Dictionary<string, object?>
            {
                ["total_files"] = inputPaths.Count,
                ["succeeded_files"] = documents.Count,
                ["failed_files"] = errors.Count,
                ["coverage_ratio"] = inputPaths.Count == 0 ? 1.0 : (double)documents.Count / inputPaths.Count,
                ["documents"] = documents,
                ["errors"] = errors,
            });
    }

    private static OfficeReceipt ReadWorkbook(string inputPath, string? sheetName, int maxRows, string capabilityId)
    {
        using var document = SpreadsheetDocument.Open(inputPath, false);
        var workbookPart = document.WorkbookPart ?? throw new InvalidOperationException("XLSX has no workbook part");
        var sharedStrings = workbookPart.SharedStringTablePart?.SharedStringTable?
            .Elements<SharedStringItem>()
            .Select(static item => item.InnerText)
            .ToList() ?? new List<string>();
        var sheets = workbookPart.Workbook.Sheets?.Elements<Sheet>().ToList() ?? new List<Sheet>();
        var cells = new List<Dictionary<string, object?>>();
        var rowTexts = new List<string>();
        foreach (var sheet in sheets)
        {
            var name = sheet.Name?.Value ?? "";
            if (!string.IsNullOrWhiteSpace(sheetName) &&
                !string.Equals(name, sheetName, StringComparison.OrdinalIgnoreCase))
            {
                continue;
            }
            if (sheet.Id?.Value is null)
            {
                continue;
            }
            var worksheetPart = (WorksheetPart)workbookPart.GetPartById(sheet.Id.Value);
            var rows = worksheetPart.Worksheet.Descendants<Row>().Take(Math.Max(maxRows, 1)).ToList();
            foreach (var row in rows)
            {
                var values = new List<string>();
                foreach (var cell in row.Elements<Cell>())
                {
                    var value = CellText(cell, sharedStrings);
                    values.Add(value);
                    if (!string.IsNullOrWhiteSpace(value))
                    {
                        cells.Add(new Dictionary<string, object?>
                        {
                            ["sheet"] = name,
                            ["row"] = row.RowIndex?.Value ?? 0,
                            ["cell_ref"] = cell.CellReference?.Value ?? "",
                            ["value"] = value,
                        });
                    }
                }
                if (values.Any(static value => !string.IsNullOrWhiteSpace(value)))
                {
                    rowTexts.Add($"{name}!{row.RowIndex}: " + string.Join("\t", values));
                }
            }
        }

        return new OfficeReceipt(
            capabilityId,
            "success",
            new Dictionary<string, object?>
            {
                ["source_path"] = Path.GetFullPath(inputPath),
                ["source_hash"] = Sha256File(inputPath),
                ["sheet_filter"] = sheetName,
                ["sheet_count"] = sheets.Count,
                ["max_rows_per_sheet"] = maxRows,
                ["cell_count"] = cells.Count,
                ["cells"] = cells,
                ["text"] = string.Join(Environment.NewLine, rowTexts),
            });
    }

    private static string CellText(Cell cell, IReadOnlyList<string> sharedStrings)
    {
        var raw = cell.CellValue?.InnerText ?? cell.InnerText ?? "";
        if (cell.DataType?.Value == CellValues.SharedString &&
            int.TryParse(raw, out var index) &&
            index >= 0 &&
            index < sharedStrings.Count)
        {
            return sharedStrings[index];
        }
        if (cell.DataType?.Value == CellValues.Boolean)
        {
            return raw == "1" ? "TRUE" : "FALSE";
        }
        return raw;
    }

    private static OfficeReceipt CreateDocx(string outputPath, string text, string? title)
    {
        Directory.CreateDirectory(Path.GetDirectoryName(Path.GetFullPath(outputPath)) ?? ".");
        var paragraphCount = SplitParagraphs(text).Count + (string.IsNullOrWhiteSpace(title) ? 0 : 1);
        using (var document = WordprocessingDocument.Create(outputPath, WordprocessingDocumentType.Document))
        {
            var mainPart = document.AddMainDocumentPart();
            mainPart.Document = new Document(new Body());
            var body = mainPart.Document.Body!;
            if (!string.IsNullOrWhiteSpace(title))
            {
                body.AppendChild(ParagraphFromText(SanitizeInlineMarkdown(title), "Title"));
            }
            AppendRenderedText(body, text);
            mainPart.Document.Save();
        }

        using var reopened = WordprocessingDocument.Open(outputPath, false);
        var validation = ValidateOpenXml(reopened);
        return new OfficeReceipt(
            "office.docx.create",
            validation.Count == 0 ? "success" : "failed",
            new Dictionary<string, object?>
            {
                ["artifact_path"] = Path.GetFullPath(outputPath),
                ["artifact_hash"] = Sha256File(outputPath),
                ["paragraph_count"] = paragraphCount,
                ["validation_errors"] = validation,
            });
    }

    private static OfficeReceipt RewriteSaveAs(string inputPath, string outputPath, string replacementText)
    {
        var sourceHashBefore = Sha256File(inputPath);
        Directory.CreateDirectory(Path.GetDirectoryName(Path.GetFullPath(outputPath)) ?? ".");
        File.Copy(inputPath, outputPath, overwrite: true);

        using (var document = WordprocessingDocument.Open(outputPath, true))
        {
            var mainPart = document.MainDocumentPart ?? throw new InvalidOperationException("DOCX has no main document part");
            var body = mainPart.Document?.Body ?? throw new InvalidOperationException("DOCX has no body");
            var section = body.Elements<SectionProperties>().LastOrDefault()?.CloneNode(deep: true);
            body.RemoveAllChildren();
            AppendRenderedText(body, replacementText);
            if (section is not null)
            {
                body.AppendChild(section);
            }
            mainPart.Document.Save();
        }

        using var rewritten = WordprocessingDocument.Open(outputPath, false);
        var validation = ValidateOpenXml(rewritten);
        return new OfficeReceipt(
            "office.docx.rewrite_save_as",
            validation.Count == 0 && sourceHashBefore == Sha256File(inputPath) ? "success" : "failed",
            new Dictionary<string, object?>
            {
                ["source_path"] = Path.GetFullPath(inputPath),
                ["artifact_path"] = Path.GetFullPath(outputPath),
                ["source_hash_before"] = sourceHashBefore,
                ["source_hash_after"] = Sha256File(inputPath),
                ["artifact_hash"] = Sha256File(outputPath),
                ["paragraph_count"] = SplitParagraphs(replacementText).Count,
                ["hash_preserve"] = sourceHashBefore == Sha256File(inputPath),
                ["validation_errors"] = validation,
            });
    }

    private static OfficeReceipt PreviewRewrite(string inputPath, string replacementText)
    {
        using var document = WordprocessingDocument.Open(inputPath, false);
        var extraction = ExtractDocx(document);
        var diff = BuildLineDiff(extraction.Paragraphs, SplitParagraphs(replacementText));
        return new OfficeReceipt(
            "office.docx.rewrite_preview",
            "success",
            new Dictionary<string, object?>
            {
                ["source_path"] = Path.GetFullPath(inputPath),
                ["source_hash"] = Sha256File(inputPath),
                ["source_paragraph_count"] = extraction.Paragraphs.Count,
                ["replacement_paragraph_count"] = SplitParagraphs(replacementText).Count,
                ["diff_summary"] = diff,
                ["mutation_performed"] = false,
            });
    }

    private static OfficeReceipt PreviewInPlaceRewrite(string inputPath, string replacementText)
    {
        var receipt = PreviewRewrite(inputPath, replacementText);
        return receipt with
        {
            CapabilityId = "office.docx.rewrite_in_place_preview",
            Data = new Dictionary<string, object?>(receipt.Data)
            {
                ["preview_kind"] = "in_place_rewrite",
                ["requires_approval"] = true,
            },
        };
    }

    private static OfficeReceipt RewriteInPlace(string inputPath, string replacementText)
    {
        var sourceHashBefore = Sha256File(inputPath);
        using (var document = WordprocessingDocument.Open(inputPath, true))
        {
            var mainPart = document.MainDocumentPart ?? throw new InvalidOperationException("DOCX has no main document part");
            var body = mainPart.Document?.Body ?? throw new InvalidOperationException("DOCX has no body");
            var section = body.Elements<SectionProperties>().LastOrDefault()?.CloneNode(deep: true);
            body.RemoveAllChildren();
            AppendRenderedText(body, replacementText);
            if (section is not null)
            {
                body.AppendChild(section);
            }
            mainPart.Document.Save();
        }

        using var rewritten = WordprocessingDocument.Open(inputPath, false);
        var validation = ValidateOpenXml(rewritten);
        return new OfficeReceipt(
            "office.docx.rewrite_in_place",
            validation.Count == 0 ? "success" : "failed",
            new Dictionary<string, object?>
            {
                ["source_path"] = Path.GetFullPath(inputPath),
                ["source_hash_before"] = sourceHashBefore,
                ["source_hash_after"] = Sha256File(inputPath),
                ["paragraph_count"] = SplitParagraphs(replacementText).Count,
                ["mutation_performed"] = true,
                ["validation_errors"] = validation,
            });
    }

    private static OfficeReceipt DiffSummary(string beforePath, string afterPath)
    {
        using var before = WordprocessingDocument.Open(beforePath, false);
        using var after = WordprocessingDocument.Open(afterPath, false);
        var beforeText = ExtractDocx(before).Paragraphs;
        var afterText = ExtractDocx(after).Paragraphs;
        return new OfficeReceipt(
            "office.docx.diff_summary",
            "success",
            new Dictionary<string, object?>
            {
                ["before_path"] = Path.GetFullPath(beforePath),
                ["after_path"] = Path.GetFullPath(afterPath),
                ["before_hash"] = Sha256File(beforePath),
                ["after_hash"] = Sha256File(afterPath),
                ["diff_summary"] = BuildLineDiff(beforeText, afterText),
            });
    }

    private static OfficeReceipt ValidateDocx(string inputPath)
    {
        using var document = WordprocessingDocument.Open(inputPath, false);
        var errors = ValidateOpenXml(document);
        return new OfficeReceipt(
            "office.docx.validate",
            errors.Count == 0 ? "success" : "failed",
            new Dictionary<string, object?>
            {
                ["source_path"] = Path.GetFullPath(inputPath),
                ["source_hash"] = Sha256File(inputPath),
                ["validation_errors"] = errors,
            });
    }

    private static OfficeReceipt SelfTest()
    {
        var root = Path.Combine(Path.GetTempPath(), $"supernova_office_worker_{DateTimeOffset.UtcNow.ToUnixTimeMilliseconds()}");
        Directory.CreateDirectory(root);
        var source = Path.Combine(root, "source.docx");
        var rewritten = Path.Combine(root, "rewritten.docx");
        CreateDocx(source, "First business paragraph.\nSecond paragraph with action item.", "Self test document");
        var read = ReadText(source);
        var preview = PreviewRewrite(source, "Rewritten first paragraph.\nRewritten second paragraph.");
        var rewrite = RewriteSaveAs(source, rewritten, "Rewritten first paragraph.\nRewritten second paragraph.");
        var diff = DiffSummary(source, rewritten);
        var inPlacePreview = PreviewInPlaceRewrite(source, "In-place first paragraph.\nIn-place second paragraph.");
        var inPlace = RewriteInPlace(source, "In-place first paragraph.\nIn-place second paragraph.");
        return new OfficeReceipt(
            "office.worker.self_test",
            read.Status == "success" && preview.Status == "success" && rewrite.Status == "success" && diff.Status == "success" && inPlacePreview.Status == "success" && inPlace.Status == "success"
                ? "success"
                : "failed",
            new Dictionary<string, object?>
            {
                ["temp_root"] = root,
                ["read_status"] = read.Status,
                ["preview_status"] = preview.Status,
                ["rewrite_status"] = rewrite.Status,
                ["diff_status"] = diff.Status,
                ["in_place_preview_status"] = inPlacePreview.Status,
                ["in_place_status"] = inPlace.Status,
            });
    }

    private static DocxExtraction ExtractDocx(WordprocessingDocument document)
    {
        var body = document.MainDocumentPart?.Document?.Body
            ?? throw new InvalidOperationException("DOCX has no body");
        var paragraphs = new List<string>();
        var headings = new List<HeadingInfo>();
        var tables = new List<TableInfo>();
        var styles = new HashSet<string>(StringComparer.Ordinal);

        foreach (var paragraph in body.Descendants<Paragraph>())
        {
            var text = paragraph.InnerText.Trim();
            if (string.IsNullOrWhiteSpace(text))
            {
                continue;
            }
            paragraphs.Add(text);
            var styleId = paragraph.ParagraphProperties?.ParagraphStyleId?.Val?.Value;
            if (!string.IsNullOrWhiteSpace(styleId))
            {
                styles.Add(styleId);
                if (styleId.StartsWith("Heading", StringComparison.OrdinalIgnoreCase))
                {
                    headings.Add(new HeadingInfo(text, styleId));
                }
            }
        }

        foreach (var table in body.Descendants<DocumentFormat.OpenXml.Wordprocessing.Table>())
        {
            var rows = new List<string[]>();
            foreach (var row in table.Elements<TableRow>())
            {
                rows.Add(row.Elements<TableCell>().Select(static cell => cell.InnerText.Trim()).ToArray());
            }
            tables.Add(new TableInfo(rows));
        }

        foreach (var runStyle in body.Descendants<RunStyle>())
        {
            if (!string.IsNullOrWhiteSpace(runStyle.Val?.Value))
            {
                styles.Add(runStyle.Val.Value);
            }
        }

        return new DocxExtraction(paragraphs, headings, tables, styles);
    }

    private static List<ValidationErrorInfo> ValidateOpenXml(WordprocessingDocument document)
    {
        var validator = new OpenXmlValidator();
        return validator.Validate(document)
            .Select(static error => new ValidationErrorInfo(
                error.Id ?? "OpenXmlValidationError",
                error.Description ?? "",
                error.Path?.XPath ?? ""))
            .ToList();
    }

    private static void AppendRenderedText(Body body, string text)
    {
        foreach (var line in SplitParagraphs(text))
        {
            var paragraph = ParagraphFromMarkdownLine(line);
            if (paragraph is not null)
            {
                body.AppendChild(paragraph);
            }
        }
    }

    private static Paragraph? ParagraphFromMarkdownLine(string line)
    {
        var trimmed = line.Trim();
        if (trimmed is "---" or "***" or "___" ||
            trimmed.StartsWith("```", StringComparison.Ordinal))
        {
            return null;
        }

        if (trimmed.StartsWith("### ", StringComparison.Ordinal))
        {
            return ParagraphFromText(SanitizeInlineMarkdown(trimmed[4..]), "Heading3");
        }

        if (trimmed.StartsWith("## ", StringComparison.Ordinal))
        {
            return ParagraphFromText(SanitizeInlineMarkdown(trimmed[3..]), "Heading2");
        }

        if (trimmed.StartsWith("# ", StringComparison.Ordinal))
        {
            return ParagraphFromText(SanitizeInlineMarkdown(trimmed[2..]), "Heading1");
        }

        if (trimmed.StartsWith("- ", StringComparison.Ordinal) ||
            trimmed.StartsWith("* ", StringComparison.Ordinal))
        {
            return ParagraphFromText("\u2022 " + SanitizeInlineMarkdown(trimmed[2..]));
        }

        return ParagraphFromText(SanitizeInlineMarkdown(trimmed));
    }

    private static Paragraph ParagraphFromText(string text, string? styleId = null)
    {
        var paragraph = new Paragraph(new DocumentFormat.OpenXml.Wordprocessing.Run(
            new DocumentFormat.OpenXml.Wordprocessing.Text(text) { Space = SpaceProcessingModeValues.Preserve }));
        if (!string.IsNullOrWhiteSpace(styleId))
        {
            paragraph.ParagraphProperties = new ParagraphProperties(
                new ParagraphStyleId { Val = styleId });
        }
        return paragraph;
    }

    private static string SanitizeInlineMarkdown(string text)
    {
        return text
            .Replace("**", "", StringComparison.Ordinal)
            .Replace("__", "", StringComparison.Ordinal)
            .Replace("`", "", StringComparison.Ordinal)
            .Trim();
    }

    private static List<string> SplitParagraphs(string text)
    {
        return text
            .Replace("\r\n", "\n", StringComparison.Ordinal)
            .Split('\n', StringSplitOptions.TrimEntries)
            .Where(static line => !string.IsNullOrWhiteSpace(line))
            .ToList();
    }

    private static object BuildLineDiff(IReadOnlyList<string> before, IReadOnlyList<string> after)
    {
        var changed = 0;
        var max = Math.Max(before.Count, after.Count);
        var samples = new List<object>();
        for (var index = 0; index < max; index++)
        {
            var oldLine = index < before.Count ? before[index] : null;
            var newLine = index < after.Count ? after[index] : null;
            if (oldLine == newLine)
            {
                continue;
            }
            changed++;
            if (samples.Count < 20)
            {
                samples.Add(new { index, before = oldLine, after = newLine });
            }
        }
        return new
        {
            before_count = before.Count,
            after_count = after.Count,
            changed_count = changed,
            samples,
        };
    }

    private static string TextInput(IReadOnlyDictionary<string, string> options)
    {
        if (options.TryGetValue("text-file", out var path))
        {
            return File.ReadAllText(path, Encoding.UTF8);
        }
        if (options.TryGetValue("text", out var text))
        {
            return text;
        }
        throw new ArgumentException("expected --text-file or --text");
    }

    private static Dictionary<string, string> ParseOptions(string[] args)
    {
        var values = new Dictionary<string, string>(StringComparer.Ordinal);
        for (var i = 0; i < args.Length; i++)
        {
            var key = args[i];
            if (!key.StartsWith("--", StringComparison.Ordinal))
            {
                throw new ArgumentException($"unexpected argument: {key}");
            }
            if (i + 1 >= args.Length)
            {
                throw new ArgumentException($"missing value for {key}");
            }
            values[key[2..]] = args[++i];
        }
        return values;
    }

    private static string Required(IReadOnlyDictionary<string, string> options, string key)
    {
        return options.TryGetValue(key, out var value) && !string.IsNullOrWhiteSpace(value)
            ? value
            : throw new ArgumentException($"missing --{key}");
    }

    private static string? Optional(IReadOnlyDictionary<string, string> options, string key)
    {
        return options.TryGetValue(key, out var value) ? value : null;
    }

    private static int? OptionalInt(IReadOnlyDictionary<string, string> options, string key)
    {
        return options.TryGetValue(key, out var value) && int.TryParse(value, out var parsed)
            ? parsed
            : null;
    }

    private static void Emit(OfficeReceipt receipt, string? outputPath)
    {
        var json = JsonSerializer.Serialize(receipt, JsonOptions);
        Console.WriteLine(json);
        if (!string.IsNullOrWhiteSpace(outputPath))
        {
            Directory.CreateDirectory(Path.GetDirectoryName(Path.GetFullPath(outputPath)) ?? ".");
            File.WriteAllText(outputPath, json, Encoding.UTF8);
        }
    }

    private static string Sha256File(string path)
    {
        using var stream = File.OpenRead(path);
        return Convert.ToHexString(SHA256.HashData(stream)).ToLowerInvariant();
    }

    private static void PrintHelp()
    {
        Console.Error.WriteLine("""
SuperNova.OfficeWorker DOCX commands:
  read-text --input <docx> [--receipt <json>]
  batch-read-text --input-list <txt> [--receipt <json>]
  read-workbook-text --input <xlsx> [--sheet <name>] [--max-rows <n>] [--receipt <json>]
  read-workbook-cells --input <xlsx> [--sheet <name>] [--max-rows <n>] [--receipt <json>]
  create-docx --output <docx> (--text-file <txt>|--text <text>) [--title <title>] [--receipt <json>]
  rewrite-save-as --input <docx> --output <docx> (--text-file <txt>|--text <text>) [--receipt <json>]
  preview-in-place-rewrite --input <docx> (--text-file <txt>|--text <text>) [--receipt <json>]
  rewrite-in-place --input <docx> (--text-file <txt>|--text <text>) [--receipt <json>]
  preview-rewrite --input <docx> (--text-file <txt>|--text <text>) [--receipt <json>]
  diff-summary --before <docx> --after <docx> [--receipt <json>]
  validate --input <docx> [--receipt <json>]
  self-test
""");
    }
}

internal sealed record OfficeReceipt(
    string CapabilityId,
    string Status,
    Dictionary<string, object?> Data);

internal sealed record DocxExtraction(
    List<string> Paragraphs,
    List<HeadingInfo> Headings,
    List<TableInfo> Tables,
    HashSet<string> StyleIds);

internal sealed record HeadingInfo(string Text, string StyleId);

internal sealed record TableInfo(List<string[]> Rows);

internal sealed record ValidationErrorInfo(string Id, string Description, string Path);
