let ticks = 0;

(globalThis as any).krusty.registerPlugin({
	onActivate() {
		ticks += 1;
	},
	tick() {
		ticks += 1;
	},
	renderText() {
		return [
			"JS/TS Demo",
			"running inside edon/libnode",
			`ticks: ${ticks}`,
			"edit src/index.ts and /plugins reload js-ts-demo",
		];
	},
});
