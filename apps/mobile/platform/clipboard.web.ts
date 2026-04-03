export async function setStringAsync(text: string): Promise<void> {
  await navigator.clipboard.writeText(text);
}

export async function getStringAsync(): Promise<string> {
  return navigator.clipboard.readText();
}
