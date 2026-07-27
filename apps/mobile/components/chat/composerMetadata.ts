export interface WorkspaceContextMetadata {
  label: string;
  hasBranch: boolean;
}

export function formatWorkspaceContextMetadata(
  directory?: string | null,
  targetBranch?: string | null,
): WorkspaceContextMetadata | null {
  const normalizedDirectory = directory?.trim();
  if (!normalizedDirectory) {
    return null;
  }

  const parts = normalizedDirectory.split(/[\\/]/).filter(Boolean);
  const project = parts.at(-1) ?? normalizedDirectory;
  const branch = targetBranch?.trim();

  return {
    label: branch ? `${project} · ${branch}` : project,
    hasBranch: Boolean(branch),
  };
}
