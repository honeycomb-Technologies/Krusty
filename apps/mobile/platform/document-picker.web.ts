interface DocumentResult {
  canceled: boolean;
  assets: { uri: string; name: string; mimeType: string }[];
}

export async function getDocumentAsync(_options?: any): Promise<DocumentResult> {
  return new Promise((resolve) => {
    const input = document.createElement('input');
    input.type = 'file';
    input.style.display = 'none';
    document.body.appendChild(input);
    const cleanup = () => { if (input.parentNode) input.parentNode.removeChild(input); };
    input.onchange = () => {
      cleanup();
      const file = input.files?.[0];
      if (!file) { resolve({ canceled: true, assets: [] }); return; }
      resolve({
        canceled: false,
        assets: [{ uri: URL.createObjectURL(file), name: file.name, mimeType: file.type || 'application/octet-stream' }],
      });
    };
    input.oncancel = () => { cleanup(); resolve({ canceled: true, assets: [] }); };
    input.click();
  });
}
