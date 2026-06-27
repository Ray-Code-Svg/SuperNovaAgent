export interface WorkspaceDialogResult {
  workspace_root?: string | null;
  status?: string | null;
}

export function selectedWorkspaceRoot(dialog: WorkspaceDialogResult | null | undefined) {
  if (dialog?.status !== "selected") return null;
  const workspaceRoot = dialog.workspace_root?.trim();
  return workspaceRoot || null;
}
