/**
 * The annotation runtime: pick, draw on and comment about elements in a page.
 *
 * It runs on two surfaces. In an Electron app it is a preload talking to the
 * main process over `ipcRenderer`. In a browser it is a script the previewed
 * page loads, talking over `postMessage` to whatever framed it. The picking,
 * drawing and capture logic is the same code in both; only the pipe differs,
 * and that is the whole of {@link AnnotationTransport}.
 *
 * A host that wires its own pipe imports {@link attachPreviewAnnotationRuntime}
 * from `./runtime`. A previewed page gets it from the Vite plugin instead,
 * which inlines the prebuilt browser bundle — see the package README.
 */
export * from './protocol.js'
export type { AnnotationListener, AnnotationTransport } from './transport.js'
export * from './types.js'
