use std::fs;
use std::io::{self, Read};
use std::path::PathBuf;
use std::time::Instant;

use serde_json::json;

use crate::data_runtime::SourceSet;
use crate::{
    file_fingerprint, json_err, to_json_value, write_store_zip, CapabilityReceipt, CapabilityToken,
    ProcessTruthStore, WorkspaceGuard,
};

#[derive(Clone, Debug)]
pub struct PackageRuntime {
    guard: WorkspaceGuard,
    truth: ProcessTruthStore,
    token: CapabilityToken,
}

impl PackageRuntime {
    pub fn new(guard: WorkspaceGuard, truth: ProcessTruthStore, token: CapabilityToken) -> Self {
        Self {
            guard,
            truth,
            token,
        }
    }

    pub fn build_zip(
        &self,
        source_set_ref: &str,
        destination_zip_path: &str,
        manifest_path: Option<&str>,
        checksums_path: Option<&str>,
        perf_notes_path: Option<&str>,
        exclude_globs: &[String],
    ) -> io::Result<CapabilityReceipt> {
        if let Some(receipt) = self.ensure_capability("package.build_zip") {
            return Ok(receipt);
        }
        let started = Instant::now();
        let source_set = self.read_source_set(source_set_ref)?;
        let selected = source_set
            .files
            .iter()
            .filter(|file| {
                !exclude_globs
                    .iter()
                    .any(|pattern| glob_like_match(&file.path, pattern))
            })
            .collect::<Vec<_>>();
        let destination = self.resolve_path(destination_zip_path)?;
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut sources = Vec::new();
        for file in &selected {
            sources.push((file.path.clone(), self.resolve_path(&file.path)?));
        }
        let entry_count = write_store_zip(self.guard.root(), &sources, &destination)?;
        let manifest_path = manifest_path.unwrap_or("PACK_MANIFEST.md");
        let checksums_path = checksums_path.unwrap_or("SHA256SUMS.txt");
        let perf_notes_path = perf_notes_path.unwrap_or("PERF_NOTES.json");
        let manifest = package_manifest_markdown(&source_set, &selected, exclude_globs);
        let checksums = package_checksums_text(&sources)?;
        let elapsed_ms = started.elapsed().as_millis();
        let perf_notes = json!({
            "capability": "package.build_zip",
            "source_set_ref": source_set_ref,
            "destination_zip_path": destination_zip_path.replace('\\', "/"),
            "entry_count": entry_count,
            "included_count": selected.len(),
            "excluded_count": source_set.file_count.saturating_sub(selected.len()),
            "elapsed_ms": elapsed_ms,
            "checksum_algorithm": "sha256",
        });
        self.write_artifact_text(manifest_path, &manifest)?;
        self.write_artifact_text(checksums_path, &checksums)?;
        self.write_artifact_text(
            perf_notes_path,
            &serde_json::to_string_pretty(&perf_notes).map_err(json_err)?,
        )?;
        let archive_fingerprint = file_fingerprint(&destination)?;
        let receipt = CapabilityReceipt {
            capability_id: "package.build_zip".to_string(),
            job_id: self.token.job_id.clone(),
            pid: self.token.pid.clone(),
            status: "success".to_string(),
            data: json!({
                "source_set_ref": source_set_ref,
                "archive_path": destination_zip_path.replace('\\', "/"),
                "artifact_path": destination_zip_path.replace('\\', "/"),
                "artifacts": [
                    destination_zip_path.replace('\\', "/"),
                    manifest_path.replace('\\', "/"),
                    checksums_path.replace('\\', "/"),
                    perf_notes_path.replace('\\', "/")
                ],
                "manifest_path": manifest_path.replace('\\', "/"),
                "checksums_path": checksums_path.replace('\\', "/"),
                "perf_notes_path": perf_notes_path.replace('\\', "/"),
                "entry_count": entry_count,
                "included_count": selected.len(),
                "excluded_count": source_set.file_count.saturating_sub(selected.len()),
                "archive_fingerprint": archive_fingerprint,
                "checksum_algorithm": "sha256",
                "elapsed_ms": elapsed_ms,
            }),
        };
        self.emit_receipt(&receipt)?;
        Ok(receipt)
    }

    fn read_source_set(&self, source_set_ref: &str) -> io::Result<SourceSet> {
        let path = self.truth.resolve_blob_ref(source_set_ref)?;
        serde_json::from_str(&fs::read_to_string(path)?).map_err(json_err)
    }

    fn resolve_path(&self, relative_path: &str) -> io::Result<PathBuf> {
        self.guard
            .resolve_workspace_path(relative_path)
            .map_err(|err| io::Error::new(io::ErrorKind::PermissionDenied, err))
    }

    fn write_artifact_text(&self, relative_path: &str, content: &str) -> io::Result<()> {
        let path = self.resolve_path(relative_path)?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, content.as_bytes())
    }

    fn ensure_capability(&self, capability_id: &str) -> Option<CapabilityReceipt> {
        if self
            .token
            .capabilities
            .iter()
            .any(|item| item == capability_id)
        {
            None
        } else {
            Some(self.blocked_receipt(capability_id, &format!("{capability_id} not granted")))
        }
    }

    fn emit_receipt(&self, receipt: &CapabilityReceipt) -> io::Result<()> {
        self.truth.append_event(
            Some(&self.token.pid),
            "capability_receipt",
            to_json_value(receipt)?,
        )?;
        Ok(())
    }

    fn blocked_receipt(&self, capability_id: &str, reason: &str) -> CapabilityReceipt {
        let receipt = CapabilityReceipt {
            capability_id: capability_id.to_string(),
            job_id: self.token.job_id.clone(),
            pid: self.token.pid.clone(),
            status: "blocked".to_string(),
            data: json!({"reason": reason}),
        };
        let _ = self.emit_receipt(&receipt);
        receipt
    }
}

fn package_manifest_markdown(
    source_set: &SourceSet,
    selected: &[&crate::data_runtime::SourceSetFile],
    exclude_globs: &[String],
) -> String {
    let mut out = String::new();
    out.push_str("# PACK_MANIFEST\n\n");
    out.push_str("## Summary\n\n");
    out.push_str(&format!(
        "- source_set_id: `{}`\n",
        source_set.source_set_id
    ));
    out.push_str(&format!(
        "- source_file_count: `{}`\n",
        source_set.file_count
    ));
    out.push_str(&format!("- included_count: `{}`\n", selected.len()));
    out.push_str(&format!(
        "- excluded_count: `{}`\n",
        source_set.file_count.saturating_sub(selected.len())
    ));
    out.push_str("- checksum_algorithm: `sha256`\n");
    if !exclude_globs.is_empty() {
        out.push_str("- exclude_globs:\n");
        for pattern in exclude_globs {
            out.push_str(&format!("  - `{pattern}`\n"));
        }
    }
    out.push_str("\n## Included Files\n\n");
    for file in selected {
        out.push_str(&format!("- `{}` ({} bytes)\n", file.path, file.size_bytes));
    }
    out
}

fn package_checksums_text(sources: &[(String, PathBuf)]) -> io::Result<String> {
    let mut out = String::new();
    out.push_str("# SHA256SUMS\n");
    out.push_str("# algorithm=sha256\n");
    for (relative_path, path) in sources {
        out.push_str(&format!("{}  {}\n", sha256_file_hex(path)?, relative_path));
    }
    Ok(out)
}

pub(crate) fn sha256_file_hex(path: &PathBuf) -> io::Result<String> {
    let mut file = fs::File::open(path)?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
    Ok(hex_lower(&sha256_bytes(&bytes)))
}

fn hex_lower(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}

fn sha256_bytes(input: &[u8]) -> [u8; 32] {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
        0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
        0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
        0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
        0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
        0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
        0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116,
        0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
        0xc67178f2,
    ];
    let mut h = [
        0x6a09e667u32,
        0xbb67ae85,
        0x3c6ef372,
        0xa54ff53a,
        0x510e527f,
        0x9b05688c,
        0x1f83d9ab,
        0x5be0cd19,
    ];
    let bit_len = (input.len() as u64) * 8;
    let mut msg = input.to_vec();
    msg.push(0x80);
    while (msg.len() % 64) != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&bit_len.to_be_bytes());
    let mut w = [0u32; 64];
    for chunk in msg.chunks(64) {
        for i in 0..16 {
            w[i] = u32::from_be_bytes([
                chunk[i * 4],
                chunk[i * 4 + 1],
                chunk[i * 4 + 2],
                chunk[i * 4 + 3],
            ]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }
        let mut a = h[0];
        let mut b = h[1];
        let mut c = h[2];
        let mut d = h[3];
        let mut e = h[4];
        let mut f = h[5];
        let mut g = h[6];
        let mut hh = h[7];
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ ((!e) & g);
            let temp1 = hh
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[i])
                .wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let temp2 = s0.wrapping_add(maj);
            hh = g;
            g = f;
            f = e;
            e = d.wrapping_add(temp1);
            d = c;
            c = b;
            b = a;
            a = temp1.wrapping_add(temp2);
        }
        h[0] = h[0].wrapping_add(a);
        h[1] = h[1].wrapping_add(b);
        h[2] = h[2].wrapping_add(c);
        h[3] = h[3].wrapping_add(d);
        h[4] = h[4].wrapping_add(e);
        h[5] = h[5].wrapping_add(f);
        h[6] = h[6].wrapping_add(g);
        h[7] = h[7].wrapping_add(hh);
    }
    let mut out = [0u8; 32];
    for (i, value) in h.iter().enumerate() {
        out[i * 4..i * 4 + 4].copy_from_slice(&value.to_be_bytes());
    }
    out
}

fn glob_like_match(path: &str, pattern: &str) -> bool {
    let pattern = pattern.replace('\\', "/");
    if pattern == "*" || pattern == "**/*" {
        return true;
    }
    if !pattern.contains('*') {
        return path == pattern || path.contains(&pattern);
    }
    let parts = pattern.split('*').collect::<Vec<_>>();
    let mut cursor = 0;
    for part in parts {
        if part.is_empty() {
            continue;
        }
        if let Some(found) = path[cursor..].find(part) {
            cursor += found + part.len();
        } else {
            return false;
        }
    }
    true
}
