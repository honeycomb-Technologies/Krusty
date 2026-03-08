<script lang="ts">
	import { onMount, onDestroy } from 'svelte';
	import { browser } from '$app/environment';

	let canvas: HTMLCanvasElement;
	let gl: WebGLRenderingContext | null = null;
	let program: WebGLProgram | null = null;
	let animationId = 0;
	let startTime = 0;
	let lastFrame = 0;
	let resolutionLoc: WebGLUniformLocation | null = null;
	let timeLoc: WebGLUniformLocation | null = null;
	let isVisible = true;
	let isTabActive = true;
	let isInitialized = false;
	let webglUnavailable = false;
	let cachedWidth = 0;
	let cachedHeight = 0;

	// 30fps target - smooth enough, saves resources
	const TARGET_FPS = 30;
	const FRAME_INTERVAL = 1000 / TARGET_FPS;

	const VS = `attribute vec2 p;void main(){gl_Position=vec4(p,0,1);}`;

	// Water turbulence shader - dark metallic aesthetic
	// Based on joltz0r's water turbulence, adapted by David Hoskins
	const FS = `
precision highp float;
uniform vec2 r;
uniform float t;

#define TAU 6.28318530718
#define MAX_ITER 5

void main() {
    float time = t * 0.25 + 23.0;
    vec2 uv = gl_FragCoord.xy / r;

    vec2 p = mod(uv * TAU, TAU) - 250.0;
    vec2 i = p;
    float c = 1.0;
    float inten = 0.005;

    for (int n = 0; n < MAX_ITER; n++) {
        float t = time * (1.0 - (3.5 / float(n + 1)));
        i = p + vec2(cos(t - i.x) + sin(t + i.y), sin(t - i.y) + cos(t + i.x));
        c += 1.0 / length(vec2(p.x / (sin(i.x + t) / inten), p.y / (cos(i.y + t) / inten)));
    }
    c /= float(MAX_ITER);
    c = 1.17 - pow(c, 1.4);
    float intensity = pow(abs(c), 8.0);

    // Dark metallic color palette (same as original)
    vec3 col1 = vec3(0.02, 0.03, 0.05);  // Near black
    vec3 col2 = vec3(0.04, 0.08, 0.12);  // Dark blue
    vec3 col3 = vec3(0.06, 0.12, 0.15);  // Teal hint

    vec3 color = mix(col1, col2, smoothstep(0.0, 0.4, intensity));
    color = mix(color, col3, smoothstep(0.4, 1.0, intensity));

    gl_FragColor = vec4(color, 1.0);
}`;

	function init(): boolean {
		if (!canvas || !browser) return false;

		try {
			gl = canvas.getContext('webgl', {
				alpha: true,
				antialias: false,
				depth: false,
				stencil: false,
				preserveDrawingBuffer: false,
				powerPreference: 'low-power',
				failIfMajorPerformanceCaveat: false
			});
		} catch { return false; }

		if (!gl) return false;

		const vs = gl.createShader(gl.VERTEX_SHADER);
		const fs = gl.createShader(gl.FRAGMENT_SHADER);
		if (!vs || !fs) return false;

		gl.shaderSource(vs, VS);
		gl.compileShader(vs);
		if (!gl.getShaderParameter(vs, gl.COMPILE_STATUS)) return false;

		gl.shaderSource(fs, FS);
		gl.compileShader(fs);
		if (!gl.getShaderParameter(fs, gl.COMPILE_STATUS)) return false;

		program = gl.createProgram();
		if (!program) return false;

		gl.attachShader(program, vs);
		gl.attachShader(program, fs);
		gl.linkProgram(program);
		if (!gl.getProgramParameter(program, gl.LINK_STATUS)) return false;

		gl.deleteShader(vs);
		gl.deleteShader(fs);

		const buf = gl.createBuffer();
		gl.bindBuffer(gl.ARRAY_BUFFER, buf);
		gl.bufferData(gl.ARRAY_BUFFER, new Float32Array([-1,-1,1,-1,-1,1,1,1]), gl.STATIC_DRAW);

		const loc = gl.getAttribLocation(program, 'p');
		gl.enableVertexAttribArray(loc);
		gl.vertexAttribPointer(loc, 2, gl.FLOAT, false, 0, 0);

		gl.useProgram(program);
		resolutionLoc = gl.getUniformLocation(program, 'r');
		timeLoc = gl.getUniformLocation(program, 't');

		startTime = performance.now();
		resize();
		return true;
	}

	function resize() {
		if (!canvas || !gl) return;
		const dpr = window.devicePixelRatio || 1;
		const w = (canvas.clientWidth * dpr) | 0;
		const h = (canvas.clientHeight * dpr) | 0;
		if (w !== cachedWidth || h !== cachedHeight) {
			cachedWidth = w;
			cachedHeight = h;
			canvas.width = w;
			canvas.height = h;
			gl.viewport(0, 0, w, h);
		}
	}

	function render(now: number) {
		// Skip if hidden
		if (!isVisible || !isTabActive) {
			animationId = requestAnimationFrame(render);
			return;
		}

		// Throttle framerate
		const delta = now - lastFrame;
		if (delta < FRAME_INTERVAL) {
			animationId = requestAnimationFrame(render);
			return;
		}
		lastFrame = now - (delta % FRAME_INTERVAL);

		if (!gl || !program) {
			animationId = requestAnimationFrame(render);
			return;
		}

		resize();
		gl.uniform2f(resolutionLoc, cachedWidth, cachedHeight);
		gl.uniform1f(timeLoc, (now - startTime) * 0.001);
		gl.drawArrays(gl.TRIANGLE_STRIP, 0, 4);

		animationId = requestAnimationFrame(render);
	}

	function handleVisibility() {
		isTabActive = !document.hidden;
	}

	function cleanup() {
		if (!browser) return;
		if (animationId) cancelAnimationFrame(animationId);
		document.removeEventListener('visibilitychange', handleVisibility);
		if (gl && program) gl.deleteProgram(program);
		gl = null;
		program = null;
	}

	onMount(() => {
		if (!browser) return;

		document.addEventListener('visibilitychange', handleVisibility);

		// IntersectionObserver for viewport visibility
		const observer = new IntersectionObserver(
			(entries) => { isVisible = entries[0]?.isIntersecting ?? true; },
			{ threshold: 0 }
		);
		observer.observe(canvas);

		if (init()) {
			isInitialized = true;
			animationId = requestAnimationFrame(render);
		} else {
			webglUnavailable = true;
		}

		return () => {
			observer.disconnect();
			cleanup();
		};
	});

	onDestroy(cleanup);
</script>

<canvas
	bind:this={canvas}
	class="pointer-events-none fixed inset-0 z-0 h-screen w-screen transition-opacity duration-300 {isInitialized ? 'opacity-100' : 'opacity-0'}"
	aria-hidden="true"
></canvas>

{#if webglUnavailable}
	<div
		class="pointer-events-none fixed inset-0 z-0"
		style="background: radial-gradient(120% 100% at 50% 10%, rgba(16, 28, 44, 0.75), rgba(6, 9, 16, 0.9));"
		aria-hidden="true"
	></div>
{/if}
