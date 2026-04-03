interface PickerResult {
  canceled: boolean;
  assets: { uri: string; fileName: string; mimeType: string; base64?: string }[];
}

function pickFile(accept: string): Promise<PickerResult> {
  return new Promise((resolve) => {
    const input = document.createElement('input');
    input.type = 'file';
    input.accept = accept;
    input.style.display = 'none';
    input.onchange = async () => {
      const file = input.files?.[0];
      if (!file) { resolve({ canceled: true, assets: [] }); return; }
      const base64 = await fileToBase64(file);
      resolve({
        canceled: false,
        assets: [{ uri: URL.createObjectURL(file), fileName: file.name, mimeType: file.type, base64 }],
      });
    };
    // User cancelled the picker
    input.oncancel = () => resolve({ canceled: true, assets: [] });
    document.body.appendChild(input);
    input.click();
    document.body.removeChild(input);
  });
}

function fileToBase64(file: File): Promise<string> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => {
      const result = reader.result as string;
      resolve(result.split(',')[1] ?? '');
    };
    reader.onerror = reject;
    reader.readAsDataURL(file);
  });
}

export async function launchImageLibraryAsync(_options?: any): Promise<PickerResult> {
  return pickFile('image/*');
}

export async function launchCameraAsync(_options?: any): Promise<PickerResult> {
  // Web camera capture via file input
  return pickFile('image/*;capture=camera');
}

export async function requestCameraPermissionsAsync(): Promise<{ granted: boolean }> {
  return { granted: true };
}
