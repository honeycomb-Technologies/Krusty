import { catalog } from "./catalog.generated";

export type PluginRuntime = "native" | "wasm" | "js";

export interface PluginCatalogEntry {
	id: string;
	name: string;
	version: string;
	publisher: string;
	package: string;
	description?: string;
	runtime: PluginRuntime;
	tags: string[];
	homepage?: string;
	repository?: string;
	official?: boolean;
}

export const catalogVersion = catalog.version;
export const plugins = catalog.plugins as unknown as PluginCatalogEntry[];
