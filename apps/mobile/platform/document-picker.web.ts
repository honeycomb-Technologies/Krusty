interface DocumentResult {
  canceled: boolean;
  assets: { uri: string; name: string; mimeType: string }[];
}

export async function getDocumentAsync(_options?: any): Promise<DocumentResult> {
  return new Promise((resolve) => {
    const input = document.createElement('input');
    input.type = 'file';
    input.style.display = 'none';
    input.onchange = () => {
      const file = input.files?.[0];
      if (!file) { resolve({ canceled: true, assets: [] }); return; }
      resolve({
        canceled: false,
        assets: [{ uri: URL.createObjectURL(file), name: file.name, mimeType: file.type || 'application/octet-stream' }],
      });
    };
    input.oncancel = () => resolve({ canceled: true, assets: [] });
    document.body.appendChild(input);
    input.click();
    document.body.removeChild(input);
  });
}
