/** The viewer's package version, set by `vite.config.ts`. */
declare const __RVT_VIEWER_VERSION__: string;

/** The decoder build the page uses by default, set by `vite.config.ts`: '32', '64' or '' for the engine's choice. */
declare const __RVT_VIEWER_BITS__: string;

/**
 * A worker bundled into the page and started from a blob URL (`?worker&inline`),
 * so that starting it fetches nothing.
 */
declare module '*?worker&inline' {
  const WorkerWrapper: new (options?: { name?: string }) => Worker;
  export default WorkerWrapper;
}
