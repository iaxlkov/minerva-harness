// Existing browser fixtures do not open the code editor; production workers are built by Vite.
export const viteWorkerStub={name:'vite-worker-stub',setup(build){build.onResolve({filter:/\?worker$/},args=>({path:args.path,namespace:'vite-worker-stub'}));build.onLoad({filter:/.*/,namespace:'vite-worker-stub'},()=>({contents:'export default class WorkerStub {}',loader:'js'}));}};
