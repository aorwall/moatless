/*
 * @t3tools/preview-annotation — the T3 Code element picker, built for the browser.
 *
 * Vendored, not hand-edited. Source: soaplabs/t3code packages/preview-annotation
 * at c4edc1cbe1850583c458b0bc1a3223a2a5ea68ca.
 * Regenerate with: node scripts/sync-t3-annotation.mjs [path-to-t3code]
 */
var T3PreviewAnnotation=(function(e){Object.defineProperty(e,Symbol.toStringTag,{value:`Module`});var t=`preview:start-pick`,n=`preview:cancel-pick`,r=`preview:element-picked`,i=`preview:annotation-captured`,a=`preview:annotation-theme`,o=`preview:human-input`,s=`preview:annotation-ready`,c=`preview:annotation-ping`,l=`t3code:preview-annotation`;function u(e){if(typeof e!=`object`||!e)return null;let t=e;return t.source!==`t3code:preview-annotation`||typeof t.channel!=`string`||!Array.isArray(t.args)?null:{source:l,channel:t.channel,args:t.args}}function d(e,t){return{source:l,channel:e,args:t}}var f=null,p=(()=>{if(f!==null)return f;try{f=window.matchMedia(`(color-gamut: p3)`).matches}catch{f=!1}return f})(),m=e=>p?`color(display-p3 0.84 0.19 0.78 / ${e})`:`rgba(210, 57, 192, ${e})`,h=1e4;m(.4),m(.05),m(.5),m(.08),m(.15);var g=[`id`,`class`,`aria-label`,`data-testid`,`role`,`name`,`title`],_=new Set([`id`,`data-testid`,`aria-label`,`href`,`src`,`alt`,`type`,`name`,`placeholder`,`role`,`for`,`action`,`method`,`title`,`disabled`,`checked`,`readonly`,`required`,`selected`,`open`]),v=`data-react-grab-frozen`,y=new Set(`display.position.top.right.bottom.left.z-index.overflow.overflow-x.overflow-y.width.height.min-width.min-height.max-width.max-height.margin-top.margin-right.margin-bottom.margin-left.padding-top.padding-right.padding-bottom.padding-left.flex-direction.flex-wrap.justify-content.align-items.align-self.align-content.flex-grow.flex-shrink.flex-basis.order.gap.row-gap.column-gap.grid-template-columns.grid-template-rows.grid-template-areas.font-family.font-size.font-weight.font-style.line-height.letter-spacing.text-align.text-decoration-line.text-decoration-style.text-transform.text-overflow.text-shadow.white-space.word-break.overflow-wrap.vertical-align.color.background-color.background-image.background-position.background-size.background-repeat.border-top-width.border-right-width.border-bottom-width.border-left-width.border-top-style.border-right-style.border-bottom-style.border-left-style.border-top-color.border-right-color.border-bottom-color.border-left-color.border-top-left-radius.border-top-right-radius.border-bottom-left-radius.border-bottom-right-radius.box-shadow.opacity.transform.filter.backdrop-filter.object-fit.object-position`.split(`.`)),b=e=>(e.tagName||``).toLowerCase(),x=`bippy-0.5.41`,S=Object.defineProperty,ee=Object.prototype.hasOwnProperty,C=()=>{},w=e=>{try{Function.prototype.toString.call(e).indexOf(`^_^`)>-1&&setTimeout(()=>{throw Error(`React is running in production mode, but dead code elimination has not been applied. Read how to correctly configure React for production: https://reactjs.org/link/perf-use-production-build`)})}catch{}},T=(e=globalThis.__REACT_DEVTOOLS_GLOBAL_HOOK__)=>!!(e&&`getFiberRoots`in e),E=!1,te,D=(e=globalThis.__REACT_DEVTOOLS_GLOBAL_HOOK__)=>E?!0:(e&&typeof e.inject==`function`&&(te=e.inject.toString()),!!te?.includes(`(injected)`)),ne=new Set,O=new Set,k=e=>{let t=new Map,n=0,r={_instrumentationIsActive:!1,_instrumentationSource:x,checkDCE:w,hasUnsupportedRendererAttached:!1,inject(e){let i=++n;return t.set(i,e),O.add(e),r._instrumentationIsActive||(r._instrumentationIsActive=!0,ne.forEach(e=>e())),i},on:C,onCommitFiberRoot:C,onCommitFiberUnmount:C,onPostCommitFiberRoot:C,renderers:t,supportsFiber:!0,supportsFlight:!0};try{S(globalThis,`__REACT_DEVTOOLS_GLOBAL_HOOK__`,{configurable:!0,enumerable:!0,get(){return r},set(t){if(t&&typeof t==`object`){let n=r.renderers;r=t,n.size>0&&(n.forEach((e,n)=>{O.add(e),t.renderers.set(n,e)}),A(e))}}});let t=window.hasOwnProperty,n=!1;S(window,`hasOwnProperty`,{configurable:!0,value:function(...e){try{if(!n&&e[0]===`__REACT_DEVTOOLS_GLOBAL_HOOK__`)return globalThis.__REACT_DEVTOOLS_GLOBAL_HOOK__=void 0,n=!0,-0}catch{}return t.apply(this,e)},writable:!0})}catch{A(e)}return r},A=e=>{e&&ne.add(e);try{let t=globalThis.__REACT_DEVTOOLS_GLOBAL_HOOK__;if(!t)return;if(!t._instrumentationSource){t.checkDCE=w,t.supportsFiber=!0,t.supportsFlight=!0,t.hasUnsupportedRendererAttached=!1,t._instrumentationSource=x,t._instrumentationIsActive=!1;let e=T(t);if(e||(t.on=C),t.renderers.size){t._instrumentationIsActive=!0,ne.forEach(e=>e());return}let n=t.inject,r=D(t);r&&!e&&(E=!0,t.inject({scheduleRefresh(){}})&&(t._instrumentationIsActive=!0)),t.inject=e=>{let i=n(e);return O.add(e),r&&t.renderers.set(i,e),t._instrumentationIsActive=!0,ne.forEach(e=>e()),i}}(t.renderers.size||t._instrumentationIsActive||D())&&e?.()}catch{}},j=()=>ee.call(globalThis,`__REACT_DEVTOOLS_GLOBAL_HOOK__`),re=e=>j()?(A(e),globalThis.__REACT_DEVTOOLS_GLOBAL_HOOK__):k(e),ie=()=>!!(typeof window<`u`&&(window.document?.createElement||window.navigator?.product===`ReactNative`)),ae=()=>{try{ie()&&re()}catch{}},oe=e=>{switch(e.tag){case 1:case 11:case 0:case 14:case 15:return!0;default:return!1}};function M(e,t,n=!1){if(!e)return null;let r=t(e);if(r instanceof Promise)return(async()=>{if(await r===!0)return e;let i=n?e.return:e.child;for(;i;){let e=await ce(i,t,n);if(e)return e;i=n?null:i.sibling}return null})();if(r===!0)return e;let i=n?e.return:e.child;for(;i;){let e=se(i,t,n);if(e)return e;i=n?null:i.sibling}return null}var se=(e,t,n=!1)=>{if(!e)return null;if(t(e)===!0)return e;let r=n?e.return:e.child;for(;r;){let e=se(r,t,n);if(e)return e;r=n?null:r.sibling}return null},ce=async(e,t,n=!1)=>{if(!e)return null;if(await t(e)===!0)return e;let r=n?e.return:e.child;for(;r;){let e=await ce(r,t,n);if(e)return e;r=n?null:r.sibling}return null},le=e=>{let t=e;return typeof t==`function`?t:typeof t==`object`&&t?le(t.type||t.render):null},N=e=>{let t=e;if(typeof t==`string`)return t;if(typeof t!=`function`&&!(typeof t==`object`&&t))return null;let n=t.displayName||t.name||null;if(n)return n;let r=le(t);return r&&(r.displayName||r.name)||null},P=()=>{let e=globalThis.__REACT_DEVTOOLS_GLOBAL_HOOK__;return!!e?._instrumentationIsActive||T(e)||D(e)},F=e=>Object.prototype.toString.call(e)===`[object Object]`&&(Object.getPrototypeOf(e)===Object.prototype||Object.getPrototypeOf(e)===null),ue=(e,t=[])=>{if(!F(e))return[{path:t,value:e}];let n=[];for(let r in e){let i=e[r],a=t.concat(r);F(i)?n.push(...ue(i,a)):n.push({path:a,value:i})}return n},I=e=>{let t=globalThis.__REACT_DEVTOOLS_GLOBAL_HOOK__;if(t?.renderers)for(let n of t.renderers.values())try{let t=n.findFiberByHostInstance?.(e);if(t)return t}catch{}if(typeof e==`object`&&e){if(`_reactRootContainer`in e)return e._reactRootContainer?._internalRoot?.current?.child;for(let t in e)if(t.startsWith(`__reactContainer$`)||t.startsWith(`__reactInternalInstance$`)||t.startsWith(`__reactFiber`))return e[t]||null}return null},de=/^[a-zA-Z][a-zA-Z\d+\-.]*:/,fe=[`rsc://`,`file:///`,`webpack-internal://`,`webpack://`,`node:`,`turbopack://`,`metro://`,`/app-pages-browser/`,`/(app-pages-browser)/`],pe=[`<anonymous>`,`eval`,``],L=/\.(jsx|tsx|ts|js)$/,R=/(\.min|bundle|chunk|vendor|vendors|runtime|polyfill|polyfills)\.(js|mjs|cjs)$|(chunk|bundle|vendor|vendors|runtime|polyfill|polyfills|framework|app|main|index)[-_.][A-Za-z0-9_-]{4,}\.(js|mjs|cjs)$|[\da-f]{8,}\.(js|mjs|cjs)$|[-_.][\da-f]{20,}\.(js|mjs|cjs)$|\/dist\/|\/build\/|\/.next\/|\/out\/|\/node_modules\/|\.webpack\.|\.vite\.|\.turbopack\./i,me=/^\?[\w~.-]+(?:=[^&#]*)?(?:&[\w~.-]+(?:=[^&#]*)?)*$/,z=/\(at [^)]+\)$/,he=/(^|@)\S+:\d+/,ge=/^\s*at .*(\S+:\d+|\(native\))/m,_e=/^(eval@)?(\[native code\])?$/,B=(e,t)=>{if(t?.includeInElement!==!1){let n=e.split(`
`),r=[];for(let e of n)if(/^\s*at\s+/.test(e)){let t=U(e,void 0)[0];t&&r.push(t)}else if(/^\s*in\s+/.test(e)){let t=e.replace(/^\s*in\s+/,``).replace(/\s*\(at .*\)$/,``);r.push({functionName:t,source:e})}else if(e.match(he)){let t=W(e,void 0)[0];t&&r.push(t)}return H(r,t)}return e.match(ge)?U(e,t):W(e,t)},V=e=>{if(!e.includes(`:`))return[e,void 0,void 0];let t=e.startsWith(`(`)&&/:\d+\)$/.test(e)?e.slice(1,-1):e,n=/(.+?)(?::(\d+))?(?::(\d+))?$/.exec(t);return n?[n[1],n[2]||void 0,n[3]||void 0]:[t,void 0,void 0]},H=(e,t)=>t&&t.slice!=null?Array.isArray(t.slice)?e.slice(t.slice[0],t.slice[1]):e.slice(0,t.slice):e,U=(e,t)=>H(e.split(`
`).filter(e=>!!e.match(ge)),t).map(e=>{let t=e;t.includes(`(eval `)&&(t=t.replace(/eval code/g,`eval`).replace(/(\(eval at [^()]*)|(,.*$)/g,``));let n=t.replace(/^\s+/,``).replace(/\(eval code/g,`(`).replace(/^.*?\s+/,``),r=n.match(/ (\(.+\)$)/);n=r?n.replace(r[0],``):n;let i=V(r?r[1]:n);return{functionName:r&&n||void 0,fileName:[`eval`,`<anonymous>`].includes(i[0])?void 0:i[0],lineNumber:i[1]?+i[1]:void 0,columnNumber:i[2]?+i[2]:void 0,source:t}}),W=(e,t)=>H(e.split(`
`).filter(e=>!e.match(_e)),t).map(e=>{let t=e;if(t.includes(` > eval`)&&(t=t.replace(/ line (\d+)(?: > eval line \d+)* > eval:\d+:\d+/g,`:$1`)),!t.includes(`@`)&&!t.includes(`:`))return{functionName:t};{let e=/(([^\n\r"\u2028\u2029]*".[^\n\r"\u2028\u2029]*"[^\n\r@\u2028\u2029]*(?:@[^\n\r"\u2028\u2029]*"[^\n\r@\u2028\u2029]*)*(?:[\n\r\u2028\u2029][^@]*)?)?[^@]*)@/,n=t.match(e),r=n&&n[1]?n[1]:void 0,i=V(t.replace(e,``));return{functionName:r,fileName:i[0],lineNumber:i[1]?+i[1]:void 0,columnNumber:i[2]?+i[2]:void 0,source:t}}}),ve=44,ye=`ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/`,be=new Uint8Array(64),xe=new Uint8Array(128);for(let e=0;e<ye.length;e++){let t=ye.charCodeAt(e);be[e]=t,xe[t]=e}function Se(e,t){let n=0,r=0,i=0;do i=xe[e.next()],n|=(i&31)<<r,r+=5;while(i&32);let a=n&1;return n>>>=1,a&&(n=-2147483648|-n),t+n}function Ce(e,t){return e.pos>=t?!1:e.peek()!==ve}var we=class{constructor(e){this.pos=0,this.buffer=e}next(){return this.buffer.charCodeAt(this.pos++)}peek(){return this.buffer.charCodeAt(this.pos)}indexOf(e){let{buffer:t,pos:n}=this,r=t.indexOf(e,n);return r===-1?t.length:r}};function Te(e){let{length:t}=e,n=new we(e),r=[],i=0,a=0,o=0,s=0,c=0;do{let e=n.indexOf(`;`),t=[],l=!0,u=0;for(i=0;n.pos<e;){let r;i=Se(n,i),i<u&&(l=!1),u=i,Ce(n,e)?(a=Se(n,a),o=Se(n,o),s=Se(n,s),Ce(n,e)?(c=Se(n,c),r=[i,a,o,s,c]):r=[i,a,o,s]):r=[i],t.push(r),n.pos++}l||Ee(t),r.push(t),n.pos=e+1}while(n.pos<=t);return r}function Ee(e){e.sort(De)}function De(e,t){return e[0]-t[0]}var Oe=/^[a-zA-Z][a-zA-Z\d+\-.]*:/,ke=/^data:application\/json[^,]+base64,/,Ae=/(?:\/\/[@#][ \t]+sourceMappingURL=([^\s'"]+?)[ \t]*$)|(?:\/\*[@#][ \t]+sourceMappingURL=([^*]+?)[ \t]*(?:\*\/)[ \t]*$)/,G=new Map,K=new Map,je=(e,t,n,r)=>{if(n<0||n>=e.length)return null;let i=e[n];if(!i||i.length===0)return null;let a=null;for(let e of i)if(e[0]<=r)a=e;else break;if(!a||a.length<4)return null;let[,o,s,c]=a;if(o===void 0||s===void 0||c===void 0)return null;let l=t[o];return l?{columnNumber:c,fileName:l,lineNumber:s+1}:null},Me=(e,t,n)=>{if(e.sections){let r=null;for(let i of e.sections)if(t>i.offset.line||t===i.offset.line&&n>=i.offset.column)r=i;else break;if(!r)return null;let i=t-r.offset.line,a=t===r.offset.line?n-r.offset.column:n;return je(r.map.mappings,r.map.sources,i,a)}return je(e.mappings,e.sources,t-1,n)},Ne=(e,t)=>{let n=t.split(`
`),r;for(let e=n.length-1;e>=0&&!r;e--){let t=n[e].match(Ae);t&&(r=t[1]||t[2])}if(!r)return null;let i=Oe.test(r);if(!(ke.test(r)||i||r.startsWith(`/`))){let t=e.split(`/`);t[t.length-1]=r,r=t.join(`/`)}return r},Pe=e=>({file:e.file,mappings:Te(e.mappings),names:e.names,sourceRoot:e.sourceRoot,sources:e.sources,sourcesContent:e.sourcesContent,version:3}),Fe=e=>{let t=e.sections.map(({map:e,offset:t})=>({map:{...e,mappings:Te(e.mappings)},offset:t})),n=new Set;for(let e of t)for(let t of e.map.sources)n.add(t);return{file:e.file,mappings:[],names:[],sections:t,sourceRoot:void 0,sources:Array.from(n),sourcesContent:void 0,version:3}},Ie=e=>{if(!e)return!1;let t=e.trim();if(!t)return!1;let n=t.match(Oe);if(!n)return!0;let r=n[0].toLowerCase();return r===`http:`||r===`https:`},Le=async(e,t=fetch)=>{if(!Ie(e))return null;let n;try{let r=await t(e);if(!r.ok)return null;n=await r.text()}catch{return null}if(!n)return null;let r=Ne(e,n);if(!r||!Ie(r))return null;try{let e=await t(r);if(!e.ok)return null;let n=await e.json();return`sections`in n?Fe(n):Pe(n)}catch{return null}},Re=async(e,t=!0,n)=>{if(t&&G.has(e))return G.get(e)??null;if(t&&K.has(e))return K.get(e);let r=Le(e,n);t&&K.set(e,r);let i=await r;return t&&K.delete(e),t&&(i===null?G.set(e,null):G.set(e,i)),i},ze=async(e,t=!0,n)=>await Promise.all(e.map(async e=>{if(!e.fileName)return e;let r=await Re(e.fileName,t,n);if(!r||typeof e.lineNumber!=`number`||typeof e.columnNumber!=`number`)return e;let i=Me(r,e.lineNumber,e.columnNumber);return i?{...e,source:i.fileName&&e.source?e.source.replace(e.fileName,i.fileName):e.source,fileName:i.fileName,lineNumber:i.lineNumber,columnNumber:i.columnNumber,isSymbolicated:!0}:e})),Be=e=>e._debugStack instanceof Error&&typeof e._debugStack?.stack==`string`,Ve=()=>{let e=re();for(let t of[...Array.from(O),...Array.from(e.renderers.values())]){let e=t.currentDispatcherRef;if(e&&typeof e==`object`)return`H`in e?e.H:e.current}return null},He=e=>{for(let t of O){let n=t.currentDispatcherRef;n&&typeof n==`object`&&(`H`in n?n.H=e:n.current=e)}},q=e=>`\n    in ${e}`,Ue=(e,t)=>{let n=q(e);return t&&(n+=` (at ${t})`),n},We=!1,Ge=(e,t)=>{if(!e||We)return``;let n=Error.prepareStackTrace;Error.prepareStackTrace=void 0,We=!0;let r=Ve();He(null);let i=console.error,a=console.warn;console.error=()=>{},console.warn=()=>{};try{let n={DetermineComponentFrameRoot(){let n;try{if(t){let t=function(){throw Error()};if(Object.defineProperty(t.prototype,"props",{set:function(){throw Error()}}),typeof Reflect==`object`&&Reflect.construct){try{Reflect.construct(t,[])}catch(e){n=e}Reflect.construct(e,[],t)}else{try{t.call()}catch(e){n=e}e.call(t.prototype)}}else{try{throw Error()}catch(e){n=e}let t=e();t&&typeof t.catch==`function`&&t.catch(()=>{})}}catch(e){if(e instanceof Error&&n instanceof Error&&typeof e.stack==`string`)return[e.stack,n.stack]}return[null,null]}};n.DetermineComponentFrameRoot.displayName=`DetermineComponentFrameRoot`,Object.getOwnPropertyDescriptor(n.DetermineComponentFrameRoot,`name`)?.configurable&&Object.defineProperty(n.DetermineComponentFrameRoot,"name",{value:`DetermineComponentFrameRoot`});let[r,i]=n.DetermineComponentFrameRoot();if(r&&i){let t=r.split(`
`),n=i.split(`
`),a=0,o=0;for(;a<t.length&&!t[a].includes(`DetermineComponentFrameRoot`);)a++;for(;o<n.length&&!n[o].includes(`DetermineComponentFrameRoot`);)o++;if(a===t.length||o===n.length)for(a=t.length-1,o=n.length-1;a>=1&&o>=0&&t[a]!==n[o];)o--;for(;a>=1&&o>=0;a--,o--)if(t[a]!==n[o]){if(a!==1||o!==1)do if(a--,o--,o<0||t[a]!==n[o]){let n=`\n${t[a].replace(` at new `,` at `)}`,r=N(e);return r&&n.includes(`<anonymous>`)&&(n=n.replace(`<anonymous>`,r)),n}while(a>=1&&o>=0);break}}}finally{We=!1,Error.prepareStackTrace=n,He(r),console.error=i,console.warn=a}let o=e?N(e):``;return o?q(o):``},Ke=(e,t)=>{let n=e.tag,r=``;switch(n){case 28:r=q(`Activity`);break;case 1:r=Ge(e.type,!0);break;case 11:r=Ge(e.type.render,!1);break;case 0:case 15:r=Ge(e.type,!1);break;case 5:case 26:case 27:r=q(e.type);break;case 16:r=q(`Lazy`);break;case 13:r=e.child!==t&&t!==null?q(`Suspense Fallback`):q(`Suspense`);break;case 19:r=q(`SuspenseList`);break;case 30:r=q(`ViewTransition`);break;default:return``}return r},qe=e=>{try{let t=``,n=e,r=null;do{t+=Ke(n,r);let e=n._debugInfo;if(e&&Array.isArray(e))for(let n=e.length-1;n>=0;n--){let r=e[n];typeof r.name==`string`&&(t+=Ue(r.name,r.env))}r=n,n=n.return}while(n);return t}catch(e){return e instanceof Error?`\nError generating stack: ${e.message}\n${e.stack}`:``}},Je=e=>{let t=Error.prepareStackTrace;Error.prepareStackTrace=void 0;let n=e;if(!n)return``;Error.prepareStackTrace=t,n.startsWith(`Error: react-stack-top-frame
`)&&(n=n.slice(29));let r=n.indexOf(`
`);if(r!==-1&&(n=n.slice(r+1)),r=Math.max(n.indexOf(`react_stack_bottom_frame`),n.indexOf(`react-stack-bottom-frame`)),r!==-1&&(r=n.lastIndexOf(`
`,r)),r!==-1)n=n.slice(0,r);else return``;return n},Ye=e=>!!(e.functionName&&e.fileName&&(e.fileName.startsWith(`rsc://`)||e.fileName.startsWith(`about://React/`))),Xe=(e,t)=>e.fileName===t.fileName&&e.lineNumber===t.lineNumber&&e.columnNumber===t.columnNumber,Ze=e=>{let t=new Map;for(let n of e)for(let e of n.stackFrames){if(!Ye(e))continue;let n=e.functionName,r=t.get(n)??[];r.some(t=>Xe(t,e))||(r.push(e),t.set(n,r))}return t},Qe=(e,t,n)=>{if(!e.functionName)return{...e,isServer:!0};let r=t.get(e.functionName);if(!r||r.length===0)return{...e,isServer:!0};let i=n.get(e.functionName)??0,a=r[i%r.length];return n.set(e.functionName,i+1),{...e,isServer:!0,fileName:a.fileName,lineNumber:a.lineNumber,columnNumber:a.columnNumber,source:e.source?.replace(`(at Server)`,`(${a.fileName}:${a.lineNumber}:${a.columnNumber})`)}},$e=e=>{let t=[];return M(e,e=>{if(!Be(e))return;let n=typeof e.type==`string`?e.type:N(e.type)||`<anonymous>`;t.push({componentName:n,stackFrames:B(Je(e._debugStack?.stack))})},!0),t},et=async(e,t=!0,n)=>{let r=$e(e),i=B(qe(e)),a=Ze(r),o=new Map;return ze(i.map(e=>(e.source?.includes(`(at Server)`)??!1)||e.source!=null&&z.test(e.source)?Qe(e,a,o):e).filter((e,t,n)=>{if(t===0)return!0;let r=n[t-1];return e.functionName!==r.functionName}),t,n)},tt=e=>{let t=e._debugSource;return t?typeof t==`object`&&!!t&&`fileName`in t&&typeof t.fileName==`string`&&`lineNumber`in t&&typeof t.lineNumber==`number`:!1},nt=async(e,t=!0,n)=>{if(tt(e))return e._debugSource||null;let r=await et(e,t,n);for(let e of r)if(e.fileName)return{fileName:e.fileName,lineNumber:e.lineNumber,columnNumber:e.columnNumber,functionName:e.functionName};return null},rt=e=>e.split(`/`).filter(Boolean).length,it=e=>e.split(`/`).filter(Boolean)[0]??null,at=e=>{let t=e.indexOf(`/`,1);if(t===-1||rt(e.slice(0,t))!==1)return e;let n=e.slice(t);if(!L.test(n)||rt(n)<2)return e;let r=it(n);return!r||r.startsWith(`@`)||r.length>4?e:n},ot=e=>{if(!e||pe.some(t=>t===e))return``;let t=e,n=t.startsWith(`http://`)||t.startsWith(`https://`);if(n)try{t=new URL(t).pathname}catch{}if(n&&(t=at(t)),t.startsWith(`about://React/`)){let e=t.slice(14),n=e.indexOf(`/`),r=e.indexOf(`:`);t=n!==-1&&(r===-1||n<r)?e.slice(n+1):e}let r=!0;for(;r;){r=!1;for(let e of fe)if(t.startsWith(e)){t=t.slice(e.length),e===`file:///`&&(t=`/${t.replace(/^\/+/,``)}`),r=!0;break}}if(de.test(t)){let e=t.match(de);e&&(t=t.slice(e[0].length))}if(t.startsWith(`//`)){let e=t.indexOf(`/`,2);t=e===-1?``:t.slice(e)}let i=t.indexOf(`?`);if(i!==-1){let e=t.slice(i);me.test(e)&&(t=t.slice(0,i))}return t},st=e=>{let t=ot(e);return!(!t||!L.test(t)||R.test(t))},ct=Symbol.for(`react.context`),lt=[],ut=null,dt=Error("Suspense Exception: This is not a real error! It's an implementation detail of `use` to interrupt the current render."),J=()=>{let e=ut;return e!==null&&(ut=e.next),e},ft=e=>e._currentValue,Y=(e,t,n,r=null)=>{lt.push({displayName:r,primitive:e,stackError:Error(),value:t,dispatcherHookName:n})},pt=e=>{if(typeof e==`object`&&e){let t=e;if(typeof t.then==`function`){let e=t;switch(e.status){case`fulfilled`:return Y(`Promise`,e.value,`Use`),e.value;case`rejected`:throw e.reason}throw Y(`Unresolved`,e,`Use`),dt}if(t.$$typeof===ct&&`_currentValue`in t){let e=t,n=ft(e);return Y(`Context (use)`,n,`Use`,e.displayName||`Context`),n}}throw Error(`An unsupported type was passed to use(): `+String(e))},mt=e=>{let t=ft(e);return Y(`Context`,t,`Context`,e.displayName||null),t},ht=e=>{let t=J(),n=t===null?typeof e==`function`?e():e:t.memoizedState;return Y(`State`,n,`State`),[n,()=>{}]},gt=(e,t,n)=>{let r=J(),i=r===null?n===void 0?t:n(t):r.memoizedState;return Y(`Reducer`,i,`Reducer`),[i,()=>{}]},_t=e=>{let t=J(),n=t===null?{current:e}:t.memoizedState;return Y(`Ref`,n.current,`Ref`),n},vt=()=>{let e=J();return Y(`CacheRefresh`,e===null?()=>{}:e.memoizedState,`CacheRefresh`),()=>{}},yt=e=>{J(),Y(`LayoutEffect`,e,`LayoutEffect`)},bt=e=>{J(),Y(`InsertionEffect`,e,`InsertionEffect`)},xt=e=>{J(),Y(`Effect`,e,`Effect`)},St=e=>{J();let t;typeof e==`object`&&e&&`current`in e&&(t=e.current),Y(`ImperativeHandle`,t,`ImperativeHandle`)},Ct=(e,t)=>{Y(`DebugValue`,typeof t==`function`?t(e):e,`DebugValue`)},wt=e=>{let t=J();return Y(`Callback`,t===null?e:t.memoizedState[0],`Callback`),e},Tt=e=>{let t=J(),n=t===null?e():t.memoizedState[0];return Y(`Memo`,n,`Memo`),n},Et=(e,t)=>{let n=J();J();let r=n===null?t():n.memoizedState;return Y(`SyncExternalStore`,r,`SyncExternalStore`),r},Dt=()=>{let e=J();J();let t=e===null?!1:e.memoizedState;return Y(`Transition`,t,`Transition`),[t,()=>{}]},Ot=e=>{let t=J(),n=t===null?e:t.memoizedState;return Y(`DeferredValue`,n,`DeferredValue`),n},kt=()=>{let e=J(),t=e===null?``:e.memoizedState;return Y(`Id`,t,`Id`),t},At=e=>[],jt=e=>{let t=J(),n=t===null?e:t.memoizedState;return Y(`Optimistic`,n,`Optimistic`),[n,()=>{}]},Mt=(e,t)=>{let n,r=null;if(e!==null){let t=e.memoizedState;if(typeof t==`object`&&t&&`then`in t&&typeof t.then==`function`){let e=t;switch(e.status){case`fulfilled`:n=e.value;break;case`rejected`:r=e.reason;break;default:r=dt,n=e}}else n=t}else n=t;return{value:n,error:r}},Nt=e=>(t,n)=>{let r=J();J(),J();let i=Error(),{value:a,error:o}=Mt(r,n);if(lt.push({displayName:null,primitive:e,stackError:i,value:a,dispatcherHookName:e}),o!==null)throw o;return[a,()=>{},!1]},Pt=Nt(`ActionState`),Ft={readContext:ft,use:pt,useCallback:wt,useContext:mt,useEffect:xt,useImperativeHandle:St,useLayoutEffect:yt,useInsertionEffect:bt,useMemo:Tt,useReducer:gt,useRef:_t,useState:ht,useDebugValue:Ct,useDeferredValue:Ot,useTransition:Dt,useSyncExternalStore:Et,useId:kt,useHostTransitionStatus:()=>{let e=ft({_currentValue:null});return Y(`HostTransitionStatus`,e,`HostTransitionStatus`),e},useFormState:Nt(`FormState`),useActionState:Pt,useOptimistic:jt,useMemoCache:At,useCacheRefresh:vt,useEffectEvent:e=>(J(),Y(`EffectEvent`,e,`EffectEvent`),e)};typeof Proxy>`u`||new Proxy(Ft,{get(e,t){if(Object.prototype.hasOwnProperty.call(e,t))return e[t];let n=Error(`Missing method in Dispatcher: `+t);throw n.name=`ReactDebugToolsUnsupportedHookError`,n}}),ae();var It=(e,t)=>e.length>t?`${e.slice(0,t)}...`:e,Lt,Rt=()=>{if(Lt!==void 0)return Lt;let e=document.querySelector(`script[src*="/_next/"]`)?.src,t=e?new URL(e).pathname:``,n=t.indexOf(`/_next/`);return Lt=n>0?t.slice(0,n):``,Lt},zt=/^(?:\.\/)?\/?\([a-z][a-z0-9-]*\)\//,Bt=e=>{let t=ot(e);return t=t.replace(zt,``),t.startsWith(`./`)&&(t=t.slice(2)),t},Vt=e=>{try{return decodeURIComponent(e)}catch{return e}},Ht=/(?:^|[/\\])node_modules[/\\]/g,Ut=/[/\\]\.vite[/\\]deps[^/\\]*[/\\]/g,Wt=/\.[mc]?[jt]sx?$/i,Gt=/^chunk-[A-Za-z0-9_-]+$/,Kt=/[/\\]/,qt=/^(.+?)@v?\d/,Jt=e=>e.split(Kt).filter(Boolean),Yt=e=>{let[t,n]=Jt(e);return!t||t.startsWith(`.`)?null:t.startsWith(`@`)?n?`${t}/${n}`:null:t},Xt=e=>{let t=Jt(e)[0];if(!t)return null;let n=t.replace(Wt,``);if(Gt.test(n))return null;if(!n.startsWith(`@`))return n;let r=n.indexOf(`_`);return r===-1?null:`${n.slice(0,r)}/${n.slice(r+1)}`},Zt=(e,t,n)=>{let r=null,i;for(;(i=t.exec(e))!==null;)r=i;return r?n(e.slice(r.index+r[0].length)):null},Qt=e=>e?.match(qt)?.[1]??null,$t=e=>{let t;try{t=new URL(e)}catch{return null}if(!t.hostname)return null;let n=Jt(t.pathname).map(Vt);for(let[e,t]of n.entries()){if(t.startsWith(`@`)){let r=Qt(n[e+1]);if(r)return`${t}/${r}`;continue}let r=Qt(t);if(r)return r}return null},en=e=>Zt(e,Ut,Xt)??Zt(e,Ht,Yt),tn=e=>{if(!e)return null;let t=ot(e);return t&&(en(Vt(t))||$t(e))||null},nn=e=>e.startsWith(`data-react-grab-`),rn=new Set([`_`,`$`,`motion.`,`styled.`,`chakra.`,`ark.`,`Primitive.`,`Slot.`]),an=new Set(`AppRouter.AppRouterAnnouncer.AppDevOverlay.AppDevOverlayErrorBoundary.ClientPageRoot.ClientSegmentRoot.DevRootHTTPAccessFallbackBoundary.ErrorBoundary.ErrorBoundaryHandler.GracefulDegradeBoundary.HTTPAccessErrorFallback.HTTPAccessFallbackBoundary.HTTPAccessFallbackErrorBoundary.HandleRedirect.Head.HistoryUpdater.HotReload.InnerLayoutRouter.InnerScrollAndFocusHandler.InnerScrollAndFocusHandlerOld.InnerScrollAndMaybeFocusHandler.InnerScrollHandlerNew.LoadableComponent.LoadingBoundary.LoadingBoundaryProvider.NotAllowedRootHTTPFallbackError.OfflineProvider.OuterLayoutRouter.RedirectBoundary.RedirectErrorBoundary.RenderFromTemplateContext.RenderValidationBoundaryAtThisLevel.ReplaySsrOnlyErrors.RootErrorBoundary.RootLevelDevOverlayElement.Router.ScrollAndFocusHandler.ScrollAndMaybeFocusHandler.SegmentBoundaryTrigger.SegmentBoundaryTriggerNode.SegmentStateProvider.SegmentTrieNode.SegmentViewNode.SegmentViewStateNode.ServerRoot.body.html`.split(`.`)),on=new Set([`Suspense`,`Fragment`,`StrictMode`,`Profiler`,`SuspenseList`]),sn=new Set([`MotionDOMComponent`]),cn=e=>{if(an.has(e)||on.has(e)||sn.has(e))return!0;for(let t of rn)if(e.startsWith(t))return!0;return!1},ln=e=>!(!e||cn(e)||e===`SlotClone`||e===`Slot`),un,dn=e=>(e&&(un=void 0),un??=typeof document<`u`&&!!(document.getElementById(`__NEXT_DATA__`)||document.querySelector(`nextjs-portal`)),un),X=e=>!(e.length<=1||cn(e)||e[0]!==e[0].toUpperCase()||e.endsWith(`Provider`)||e.endsWith(`Context`)),fn=[`about://React/`,`rsc://React/`],pn=e=>fn.some(t=>e.startsWith(t)),mn=e=>{for(let t of fn){if(!e.startsWith(t))continue;let n=e.indexOf(`/`,t.length);if(n===-1)continue;let r=n+1,i=e.lastIndexOf(`?`);return Vt(i>r?e.slice(r,i):e.slice(r))}return e},hn=async e=>{let t=[],n=[];for(let r=0;r<e.length;r++){let i=e[r];!i.isServer||!i.fileName||(t.push(r),n.push({file:mn(i.fileName),methodName:i.functionName??`<unknown>`,line1:i.lineNumber??null,column1:i.columnNumber??null,arguments:[]}))}if(n.length===0)return e;let r=new AbortController,i=setTimeout(()=>r.abort(),5e3);try{let i=await fetch(`${Rt()}/__nextjs_original-stack-frames`,{method:`POST`,headers:{"Content-Type":`application/json`},body:JSON.stringify({frames:n,isServer:!0,isEdgeServer:!1,isAppDirectory:!0}),signal:r.signal});if(!i.ok)return e;let a=await i.json(),o=[...e];for(let n=0;n<t.length;n++){let r=a[n];if(r?.status!==`fulfilled`)continue;let i=r.value?.originalStackFrame;if(!i?.file||i.ignored)continue;let s=t[n];o[s]={...e[s],fileName:i.file,lineNumber:i.line1??void 0,columnNumber:i.column1??void 0,isSymbolicated:!0}}return o}catch{return e}finally{clearTimeout(i)}},gn=e=>{let t=new Map;return M(e,e=>{if(!Be(e))return!1;let n=Je(e._debugStack.stack);if(!n)return!1;for(let e of B(n))!e.functionName||!e.fileName||pn(e.fileName)&&(t.has(e.functionName)||t.set(e.functionName,{...e,isServer:!0}));return!1},!0),t},_n=(e,t)=>{if(!t.some(e=>e.isServer&&!e.fileName&&e.functionName))return t;let n=gn(e);return n.size===0?t:t.map(e=>{if(!e.isServer||e.fileName||!e.functionName)return e;let t=n.get(e.functionName);return t?{...e,fileName:t.fileName,lineNumber:t.lineNumber,columnNumber:t.columnNumber}:e})},vn=e=>{if(!P())return e;let t=e;for(;t;){if(I(t))return t;t=t.parentElement}return e},yn=new WeakMap,bn=async e=>{try{let t=I(e);if(!t)return null;let n=await et(t);return dn()?await hn(_n(t,n)):n}catch{return null}},xn=e=>{if(!P())return Promise.resolve([]);let t=vn(e),n=yn.get(t);if(n)return n;let r=bn(t);return yn.set(t,r),r},Sn=e=>{if(!e||!oe(e))return null;let t=N(e.type);return t&&X(t)?t:null},Cn=async e=>{let t=I(vn(e));if(!t)return null;try{let e=await nt(t);return!e?.fileName||!st(e.fileName)?null:{filePath:Bt(e.fileName),lineNumber:e.lineNumber??null,columnNumber:e.columnNumber??null,componentName:(e.functionName&&X(e.functionName)?e.functionName:null)??Sn(t._debugOwner)}}catch{return null}},wn=async e=>{let t=await Cn(e);if(t)return t;let n=await xn(e);if(!n||n.length===0)return null;let r=n.filter(e=>e.fileName&&st(e.fileName)),i=r.find(e=>e.functionName&&X(e.functionName))??r[0];return i?.fileName?{filePath:Bt(i.fileName),lineNumber:i.lineNumber??null,columnNumber:i.columnNumber??null,componentName:i.functionName&&X(i.functionName)?i.functionName:null}:null},Tn=e=>{if(!P())return null;let t=I(vn(e));if(!t)return null;let n=t.return;for(;n;){if(oe(n)){let e=N(n.type);if(e&&ln(e))return e}n=n.return}return null},En=e=>e?e.some(e=>!!(e.fileName&&st(e.fileName)||e.isServer&&(!e.functionName||X(e.functionName))||e.functionName&&X(e.functionName))):!1,Dn=(e,t)=>{if(!P())return[];let n=I(e);if(!n)return[];let r=[];return M(n,e=>{if(r.length>=t)return!0;if(oe(e)){let t=N(e.type);t&&ln(t)&&r.push(t)}return!1},!0),r},On=[`/src/app/`,`/src/pages/`,`/app/`,`/pages/`],kn=(e,t)=>{let n=Bt(e);if(!t||!n.startsWith(`/`))return n;for(let e of On){let t=n.indexOf(e);if(t!==-1)return`/./${n.slice(t+1)}`}return n},An=(e,t)=>{let n=kn(e.filePath,t),r=t&&e.lineNumber?`${n}:${e.lineNumber}${e.columnNumber?`:${e.columnNumber}`:``}`:n;return e.componentName?`\n  in ${e.componentName} (at ${r})`:`\n  in ${r}`},jn=(e,t={},n=null)=>{let{maxLines:r=3}=t,i=dn(),a=[],o=null,s=!1;n&&a.push(An(n,i));let c=(e,t)=>{a.push(e),o=t};for(let t of e){if(a.length>=r)break;let e=t.fileName&&st(t.fileName)?t.fileName:null,l=e?null:tn(t.fileName);if(l&&l===o)continue;let u=t.functionName&&X(t.functionName)?t.functionName:null;if(!s&&u&&u===n?.componentName){s=!0;continue}if(t.isServer&&!e&&(u||!t.functionName)){let e=l?`${l} at Server`:`at Server`;c(`\n  in ${u??`<anonymous>`} (${e})`,l);continue}if(!e&&u){c(l?`\n  in ${u} (${l})`:`\n  in ${u}`,l);continue}e&&c(An({componentName:u,filePath:e,lineNumber:t.lineNumber??null,columnNumber:t.columnNumber??null},i),null)}return a.join(``)},Mn=async(e,t={})=>{let n=t.maxLines??3,r=await Cn(e),i=await xn(e);if(i&&En(i))return jn(i,t,r);if(r)return An(r,dn());let a=Dn(vn(e),n);return a.length>0?a.map(e=>`\n  in ${e}`).join(``):``},Nn=async(e,t={})=>{let n=vn(e),r=Hn(n),i=await Mn(n,t);return i?`${r}${i}`:Pn(n)},Pn=e=>{if(!(e instanceof HTMLElement))return Vn(e);let t=b(e),n=Rn(e),r=It(zn(e),100);return r.length>0?`<${t}${n}>\n  ${r}\n</${t}>`:`<${t}${n} />`},Fn=e=>It(e,15),In=(e,t={})=>{let{truncate:n=!0,maxAttrs:r=3}=t,i=[];for(let t of g){if(i.length>=r)break;let a=e.getAttribute(t);if(a){let e=n?Fn(a):a;i.push(`${t}="${e}"`)}}return i.length>0?` ${i.join(` `)}`:``},Ln=e=>e===`class`||e===`className`||e===`style`,Rn=e=>{let t=[],n=[],r=``;for(let{name:i,value:a}of e.attributes)if(!nn(i)){if(Ln(i)){i!==`style`&&a&&(r=` class="${Fn(a)}"`);continue}_.has(i)?t.push(a?` ${i}="${a}"`:` ${i}`):a&&n.push(` ${i}="${Fn(a)}"`)}return t.join(``)+n.join(``)+r},zn=e=>{let t=``;for(let n of e.childNodes)if(n.nodeType===Node.TEXT_NODE){let e=n.textContent?.trim()??``;e&&(t+=(t?` `:``)+e)}return t},Bn=e=>e.length===0?``:e.length<=2?e.map(e=>`<${b(e)} ...>`).join(`
  `):`(${e.length} elements)`,Vn=e=>{let t=b(e);if(!(e instanceof HTMLElement))return`<${t}${In(e,{truncate:!1,maxAttrs:g.length})} />`;let n=Rn(e),r=It(zn(e),100);return r?`<${t}${n}>${r}</${t}>`:`<${t}${n} />`},Hn=e=>{let t=b(e),n=Rn(e),r=zn(e),i=[],a=[],o=!1;for(let t of e.childNodes)t.nodeType!==Node.COMMENT_NODE&&(t.nodeType===Node.TEXT_NODE?t.textContent&&t.textContent.trim().length>0&&(o=!0):t instanceof Element&&(o?a.push(t):i.push(t)));let s=``,c=Bn(i);c&&(s+=`\n  ${c}`),r.length>0&&(s+=`\n  ${It(r,100)}`);let l=Bn(a);return l&&(s+=`\n  ${l}`),s.length>0?`<${t}${n}>${s}\n</${t}>`:`<${t}${n} />`},Un=typeof window<`u`,Wn=e=>0,Gn=e=>{},Kn=Un?(Object.getOwnPropertyDescriptor(Window.prototype,`requestAnimationFrame`)?.value??window.requestAnimationFrame).bind(window):Wn,qn=Un?(Object.getOwnPropertyDescriptor(Window.prototype,`cancelAnimationFrame`)?.value??window.cancelAnimationFrame).bind(window):Gn,Jn=new Map,Yn=new WeakSet,Xn=new Map,Zn=new Map,Qn=e=>!!Yn.has(e);typeof window<`u`&&(window.requestAnimationFrame=e=>Qn(e)?Kn(t=>{e(t)}):Kn(e),window.cancelAnimationFrame=e=>{if(Jn.has(e)){Jn.delete(e);return}let t=Zn.get(e);if(t!==void 0){qn(t.nativeId),Zn.delete(e);return}let n=Xn.get(e);if(n!==void 0){Jn.delete(n),Xn.delete(e);return}qn(e)}),`${v}${v}`;var $n=class extends Error{constructor(e){super(e),this.name=`ReactGrabError`}},er=class extends $n{constructor(){super(`Can't generate CSS selector for non-element node type.`),this.name=`NonElementNodeError`}},tr=class extends $n{constructor(e){super(`Timeout: Can't find a unique selector after ${e}ms`),this.name=`SelectorTimeoutError`,this.timeoutMs=e}},nr=class extends $n{constructor(){super(`Selector was not found.`),this.name=`SelectorNotFoundError`}},rr=new Set([`role`,`name`,`aria-label`,`rel`,`href`]),ir=e=>{if(!/^[a-z-]{3,}$/i.test(e))return!1;let t=e.split(/-|[A-Z]/);for(let e of t)if(e.length<=2||/[^aeiou]{4,}/i.test(e))return!1;return!0},ar=(e,t)=>{let n=rr.has(e)||e.startsWith(`data-`)&&ir(e),r=ir(t)&&t.length<100||t.startsWith(`#`)&&ir(t.slice(1));return n&&r},or=e=>{let t=e[0].name;for(let n=1;n<e.length;n++)t=`${e[n].name} > ${t}`;return t},sr=e=>{let t=0;for(let n of e)t+=n.penalty;return t},cr=(e,t)=>sr(e)-sr(t),lr=(e,t)=>{let n=e.parentNode;if(!n)return;let r=n.firstChild;if(!r)return;let i=0;for(;r&&(r.nodeType===Node.ELEMENT_NODE&&(t===void 0||r.tagName.toLowerCase()===t)&&i++,r!==e);)r=r.nextSibling;return i},ur=(e,t)=>e===`html`?`html`:`${e}:nth-child(${t})`,dr=(e,t)=>e===`html`?`html`:`${e}:nth-of-type(${t})`,fr=(e,t)=>{let n=[],r=e.getAttribute(`id`),i=e.tagName.toLowerCase();r&&ir(r)&&n.push({name:`#${CSS.escape(r)}`,penalty:0});for(let t of e.classList)ir(t)&&n.push({name:`.${CSS.escape(t)}`,penalty:1});for(let r of e.attributes)t(r.name,r.value)&&n.push({name:`[${CSS.escape(r.name)}="${CSS.escape(r.value)}"]`,penalty:2});n.push({name:i,penalty:5});let a=lr(e,i);a!==void 0&&n.push({name:dr(i,a),penalty:10});let o=lr(e);return o!==void 0&&n.push({name:ur(i,o),penalty:50}),n},pr=(e,t=h,n=[])=>{if(t<=0)return[];if(e.length===0)return[n];let r=[];for(let i of e[0]){let a=t-r.length;if(a<=0)break;r.push(...pr(e.slice(1),a,[...n,i]))}return r},mr=(e,t)=>{let n=t.getRootNode?.();return n instanceof ShadowRoot?n:e.nodeType===Node.DOCUMENT_NODE?e:e.ownerDocument},hr=(e,t)=>t.querySelectorAll(or(e)).length===1,gr=(e,t)=>{let n=e,r=[];for(;n&&n!==t;){let e=n.tagName.toLowerCase(),t=lr(n,e);if(t===void 0)return;r.push({name:dr(e,t),penalty:10}),n=n.parentElement}return hr(r,t)?r:void 0},_r=(e,t,n,r)=>{if(e.nodeType!==Node.ELEMENT_NODE)throw new er;if(e.tagName.toLowerCase()===`html`)return`html`;let i=mr(t,e),a=Date.now(),o=[],s=e,c=0,l;for(;s&&s!==i&&!l;)if(o.push(fr(s,r)),s=s.parentElement,c++,c>=3){let t=pr(o);t.sort(cr);for(let r of t){if(Date.now()-a>n){let t=gr(e,i);if(!t)throw new tr(n);return or(t)}if(hr(r,i)){l=r;break}}}if(!l&&c<3){let e=pr(o);e.sort(cr);for(let t of e){if(Date.now()-a>n)break;if(hr(t,i)){l=t;break}}}if(!l)throw new nr;return or(l)},vr=e=>typeof CSS<`u`&&typeof CSS.escape==`function`?CSS.escape(e):e.replace(/[^a-zA-Z0-9_-]/g,e=>`\\${e}`),yr=e=>e.ownerDocument.body??e.ownerDocument.documentElement,br=new Set([`data-testid`,`data-test-id`,`data-test`,`data-cy`,`data-qa`,`aria-label`,`role`,`name`,`title`,`alt`]),xr=e=>e.length>0&&e.length<=120,Sr=(e,t)=>{try{let n=e.ownerDocument.querySelectorAll(t);return n.length===1&&n[0]===e}catch{return!1}},Cr=e=>{if(e instanceof HTMLElement&&e.id){let t=`#${vr(e.id)}`;if(Sr(e,t))return t}for(let t of br){let n=e.getAttribute(t);if(!n||!xr(n))continue;let r=`[${t}=${JSON.stringify(n)}]`;if(Sr(e,r))return r;let i=`${e.tagName.toLowerCase()}${r}`;if(Sr(e,i))return i}return null},wr=e=>{let t=[],n=yr(e),r=e;for(;r;){if(r instanceof HTMLElement&&r.id){t.unshift(`#${vr(r.id)}`);break}let e=r.parentElement;if(!e){t.unshift(r.tagName.toLowerCase());break}let i=Array.from(e.children).indexOf(r),a=i>=0?i+1:1;if(t.unshift(`${r.tagName.toLowerCase()}:nth-child(${a})`),e===n){t.unshift(n.tagName.toLowerCase());break}r=e}return t.join(` > `)},Tr=(e,t=!0)=>{let n=Cr(e);if(n)return n;if(t)try{let t=_r(e,yr(e),200,(e,t)=>ar(e,t)||br.has(e)&&xr(t));if(t)return t}catch{}return wr(e)},Er=new Map([`top`,`right`,`bottom`,`left`].flatMap(e=>[[`border-${e}-style`,e],[`border-${e}-color`,e]])),Dr=null,Or=new Map,kr=()=>Dr||(Dr=document.createElement(`iframe`),Dr.style.cssText=`position:fixed;left:-9999px;width:0;height:0;border:none;visibility:hidden;`,document.body.appendChild(Dr),Dr),Ar=e=>{let t=Or.get(e);if(t)return t;let n=kr(),r=n.contentDocument,i=r.createElement(e);r.body.appendChild(i);let a=n.contentWindow.getComputedStyle(i),o=new Map;for(let e of y){let t=a.getPropertyValue(e);t&&o.set(e,t)}return i.remove(),Or.set(e,o),o},jr=(e,t)=>{let n=Er.get(e);if(!n)return!1;let r=t.getPropertyValue(`border-${n}-width`);return r===`0px`||r===`0`},Mr=e=>{let t=Ar(e.tagName.toLowerCase()),n=getComputedStyle(e),r=[];for(let e of y){let i=n.getPropertyValue(e);i&&i!==t.get(e)&&(jr(e,n)||r.push(`${e}: ${i};`))}let i=e.getAttribute(`class`)?.trim(),a=r.join(`
`);return i?a?`className: ${i}\n\n${a}`:`className: ${i}`:a},Nr=async e=>{let[t,n,r]=await Promise.all([Nn(e),wn(e),xn(e).then(e=>e??[])]),i=await Mn(e),a=Hn(e),o=Tn(e),s=I(e),c=Tr(e),l=Mr(e);return{element:e,snippet:t,htmlPreview:a,stackString:i,stack:r,componentName:o,filePath:n?.filePath??null,lineNumber:n?.lineNumber??null,columnNumber:n?.columnNumber??null,fiber:s,selector:c,styles:l}},Pr=`/*! tailwindcss v4.3.0 | MIT License | https://tailwindcss.com */
@layer properties;
:root, :host {
  --spacing: 0.25rem;
  --text-xs: 0.75rem;
  --text-xs--line-height: calc(1 / 0.75);
  --text-sm: 0.875rem;
  --text-sm--line-height: calc(1.25 / 0.875);
  --text-lg: 1.125rem;
  --text-lg--line-height: calc(1.75 / 1.125);
  --font-weight-medium: 500;
  --font-weight-semibold: 600;
  --font-weight-bold: 700;
  --blur-xl: 24px;
  --default-font-family: var(--t3-font-sans);
  --default-mono-font-family: var(--t3-font-mono);
}
*, ::after, ::before, ::backdrop, ::file-selector-button {
  box-sizing: border-box;
  margin: 0;
  padding: 0;
  border: 0 solid;
}
html, :host {
  line-height: 1.5;
  -webkit-text-size-adjust: 100%;
  tab-size: 4;
  font-family: var(--default-font-family, ui-sans-serif, system-ui, sans-serif, 'Apple Color Emoji', 'Segoe UI Emoji', 'Segoe UI Symbol', 'Noto Color Emoji');
  font-feature-settings: var(--default-font-feature-settings, normal);
  font-variation-settings: var(--default-font-variation-settings, normal);
  -webkit-tap-highlight-color: transparent;
}
hr {
  height: 0;
  color: inherit;
  border-top-width: 1px;
}
abbr:where([title]) {
  -webkit-text-decoration: underline dotted;
  text-decoration: underline dotted;
}
h1, h2, h3, h4, h5, h6 {
  font-size: inherit;
  font-weight: inherit;
}
a {
  color: inherit;
  -webkit-text-decoration: inherit;
  text-decoration: inherit;
}
b, strong {
  font-weight: bolder;
}
code, kbd, samp, pre {
  font-family: var(--default-mono-font-family, ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, 'Liberation Mono', 'Courier New', monospace);
  font-feature-settings: var(--default-mono-font-feature-settings, normal);
  font-variation-settings: var(--default-mono-font-variation-settings, normal);
  font-size: 1em;
}
small {
  font-size: 80%;
}
sub, sup {
  font-size: 75%;
  line-height: 0;
  position: relative;
  vertical-align: baseline;
}
sub {
  bottom: -0.25em;
}
sup {
  top: -0.5em;
}
table {
  text-indent: 0;
  border-color: inherit;
  border-collapse: collapse;
}
:-moz-focusring {
  outline: auto;
}
progress {
  vertical-align: baseline;
}
summary {
  display: list-item;
}
ol, ul, menu {
  list-style: none;
}
img, svg, video, canvas, audio, iframe, embed, object {
  display: block;
  vertical-align: middle;
}
img, video {
  max-width: 100%;
  height: auto;
}
button, input, select, optgroup, textarea, ::file-selector-button {
  font: inherit;
  font-feature-settings: inherit;
  font-variation-settings: inherit;
  letter-spacing: inherit;
  color: inherit;
  border-radius: 0;
  background-color: transparent;
  opacity: 1;
}
:where(select:is([multiple], [size])) optgroup {
  font-weight: bolder;
}
:where(select:is([multiple], [size])) optgroup option {
  padding-inline-start: 20px;
}
::file-selector-button {
  margin-inline-end: 4px;
}
::placeholder {
  opacity: 1;
}
@supports (not (-webkit-appearance: -apple-pay-button))  or (contain-intrinsic-size: 1px) {
  ::placeholder {
    color: currentcolor;
    @supports (color: color-mix(in lab, red, red)) {
      color: color-mix(in oklab, currentcolor 50%, transparent);
    }
  }
}
textarea {
  resize: vertical;
}
::-webkit-search-decoration {
  -webkit-appearance: none;
}
::-webkit-date-and-time-value {
  min-height: 1lh;
  text-align: inherit;
}
::-webkit-datetime-edit {
  display: inline-flex;
}
::-webkit-datetime-edit-fields-wrapper {
  padding: 0;
}
::-webkit-datetime-edit, ::-webkit-datetime-edit-year-field, ::-webkit-datetime-edit-month-field, ::-webkit-datetime-edit-day-field, ::-webkit-datetime-edit-hour-field, ::-webkit-datetime-edit-minute-field, ::-webkit-datetime-edit-second-field, ::-webkit-datetime-edit-millisecond-field, ::-webkit-datetime-edit-meridiem-field {
  padding-block: 0;
}
::-webkit-calendar-picker-indicator {
  line-height: 1;
}
:-moz-ui-invalid {
  box-shadow: none;
}
button, input:where([type='button'], [type='reset'], [type='submit']), ::file-selector-button {
  appearance: button;
}
::-webkit-inner-spin-button, ::-webkit-outer-spin-button {
  height: auto;
}
[hidden]:where(:not([hidden='until-found'])) {
  display: none !important;
}
.pointer-events-auto {
  pointer-events: auto;
}
.pointer-events-none {
  pointer-events: none;
}
.absolute {
  position: absolute;
}
.fixed {
  position: fixed;
}
.inset-0 {
  inset: calc(var(--spacing) * 0);
}
.top-1\\/2 {
  top: calc(1 / 2 * 100%);
}
.top-2\\.5 {
  top: calc(var(--spacing) * 2.5);
}
.right-2 {
  right: calc(var(--spacing) * 2);
}
.left-1\\/2 {
  left: calc(1 / 2 * 100%);
}
.z-1 {
  z-index: 1;
}
.block {
  display: block;
}
.flex {
  display: flex;
}
.grid {
  display: grid;
}
.hidden {
  display: none;
}
.inline-flex {
  display: inline-flex;
}
.h-7 {
  height: calc(var(--spacing) * 7);
}
.h-8 {
  height: calc(var(--spacing) * 8);
}
.max-h-24 {
  max-height: calc(var(--spacing) * 24);
}
.max-h-\\[calc\\(100vh-16px\\)\\] {
  max-height: calc(100vh - 16px);
}
.max-h-\\[min\\(176px\\,calc\\(100vh-180px\\)\\)\\] {
  max-height: min(176px, calc(100vh - 180px));
}
.min-h-7 {
  min-height: calc(var(--spacing) * 7);
}
.min-h-8 {
  min-height: calc(var(--spacing) * 8);
}
.w-6 {
  width: calc(var(--spacing) * 6);
}
.w-8 {
  width: calc(var(--spacing) * 8);
}
.w-\\[min\\(360px\\,calc\\(100vw-16px\\)\\)\\] {
  width: min(360px, calc(100vw - 16px));
}
.w-full {
  width: 100%;
}
.max-w-70 {
  max-width: calc(var(--spacing) * 70);
}
.min-w-0 {
  min-width: calc(var(--spacing) * 0);
}
.flex-1 {
  flex: 1;
}
.shrink-0 {
  flex-shrink: 0;
}
.-translate-x-1\\/2 {
  --tw-translate-x: calc(calc(1 / 2 * 100%) * -1);
  translate: var(--tw-translate-x) var(--tw-translate-y);
}
.-translate-y-1\\/2 {
  --tw-translate-y: calc(calc(1 / 2 * 100%) * -1);
  translate: var(--tw-translate-x) var(--tw-translate-y);
}
.cursor-grab {
  cursor: grab;
}
.cursor-pointer {
  cursor: pointer;
}
.resize {
  resize: both;
}
.resize-none {
  resize: none;
}
.appearance-none {
  appearance: none;
}
.grid-cols-\\[22px_minmax\\(0\\,1fr\\)\\] {
  grid-template-columns: 22px minmax(0,1fr);
}
.grid-cols-\\[82px_minmax\\(0\\,1fr\\)\\] {
  grid-template-columns: 82px minmax(0,1fr);
}
.flex-col {
  flex-direction: column;
}
.items-center {
  align-items: center;
}
.items-start {
  align-items: flex-start;
}
.justify-center {
  justify-content: center;
}
.gap-0\\.5 {
  gap: calc(var(--spacing) * 0.5);
}
.gap-1 {
  gap: calc(var(--spacing) * 1);
}
.gap-2 {
  gap: calc(var(--spacing) * 2);
}
.overflow-auto {
  overflow: auto;
}
.overflow-hidden {
  overflow: hidden;
}
.overflow-y-hidden {
  overflow-y: hidden;
}
.rounded-lg {
  border-radius: var(--t3-radius);
}
.rounded-md {
  border-radius: calc(var(--t3-radius) - 2px);
}
.rounded-xl {
  border-radius: calc(var(--t3-radius) + 4px);
}
.border {
  border-style: var(--tw-border-style);
  border-width: 1px;
}
.border-0 {
  border-style: var(--tw-border-style);
  border-width: 0px;
}
.border-t {
  border-top-style: var(--tw-border-style);
  border-top-width: 1px;
}
.border-b {
  border-bottom-style: var(--tw-border-style);
  border-bottom-width: 1px;
}
.border-border {
  border-color: var(--t3-border);
}
.border-input {
  border-color: var(--t3-input);
}
.border-primary {
  border-color: var(--t3-primary);
}
.border-transparent {
  border-color: transparent;
}
.border-b-transparent {
  border-bottom-color: transparent;
}
.bg-background {
  background-color: var(--t3-background);
}
.bg-muted {
  background-color: var(--t3-muted);
}
.bg-muted\\/40 {
  background-color: var(--t3-muted);
  @supports (color: color-mix(in lab, red, red)) {
    background-color: color-mix(in oklab, var(--t3-muted) 40%, transparent);
  }
}
.bg-popover\\/95 {
  background-color: var(--t3-popover);
  @supports (color: color-mix(in lab, red, red)) {
    background-color: color-mix(in oklab, var(--t3-popover) 95%, transparent);
  }
}
.bg-popover\\/96 {
  background-color: var(--t3-popover);
  @supports (color: color-mix(in lab, red, red)) {
    background-color: color-mix(in oklab, var(--t3-popover) 96%, transparent);
  }
}
.bg-primary {
  background-color: var(--t3-primary);
}
.bg-primary\\/10 {
  background-color: var(--t3-primary);
  @supports (color: color-mix(in lab, red, red)) {
    background-color: color-mix(in oklab, var(--t3-primary) 10%, transparent);
  }
}
.bg-transparent {
  background-color: transparent;
}
.p-0 {
  padding: calc(var(--spacing) * 0);
}
.p-1 {
  padding: calc(var(--spacing) * 1);
}
.p-2 {
  padding: calc(var(--spacing) * 2);
}
.px-0 {
  padding-inline: calc(var(--spacing) * 0);
}
.px-1 {
  padding-inline: calc(var(--spacing) * 1);
}
.px-2 {
  padding-inline: calc(var(--spacing) * 2);
}
.px-2\\.5 {
  padding-inline: calc(var(--spacing) * 2.5);
}
.px-3 {
  padding-inline: calc(var(--spacing) * 3);
}
.py-1 {
  padding-block: calc(var(--spacing) * 1);
}
.py-1\\.5 {
  padding-block: calc(var(--spacing) * 1.5);
}
.py-2 {
  padding-block: calc(var(--spacing) * 2);
}
.font-mono {
  font-family: var(--t3-font-mono);
}
.font-sans {
  font-family: var(--t3-font-sans);
}
.text-lg {
  font-size: var(--text-lg);
  line-height: var(--tw-leading, var(--text-lg--line-height));
}
.text-sm {
  font-size: var(--text-sm);
  line-height: var(--tw-leading, var(--text-sm--line-height));
}
.text-xs {
  font-size: var(--text-xs);
  line-height: var(--tw-leading, var(--text-xs--line-height));
}
.leading-5 {
  --tw-leading: calc(var(--spacing) * 5);
  line-height: calc(var(--spacing) * 5);
}
.font-bold {
  --tw-font-weight: var(--font-weight-bold);
  font-weight: var(--font-weight-bold);
}
.font-medium {
  --tw-font-weight: var(--font-weight-medium);
  font-weight: var(--font-weight-medium);
}
.font-semibold {
  --tw-font-weight: var(--font-weight-semibold);
  font-weight: var(--font-weight-semibold);
}
.text-foreground {
  color: var(--t3-foreground);
}
.text-muted-foreground {
  color: var(--t3-muted-foreground);
}
.text-popover-foreground {
  color: var(--t3-popover-foreground);
}
.text-primary {
  color: var(--t3-primary);
}
.text-primary-foreground {
  color: var(--t3-primary-foreground);
}
.shadow-2xl {
  --tw-shadow: 0 25px 50px -12px var(--tw-shadow-color, rgb(0 0 0 / 0.25));
  box-shadow: var(--tw-inset-shadow), var(--tw-inset-ring-shadow), var(--tw-ring-offset-shadow), var(--tw-ring-shadow), var(--tw-shadow);
}
.shadow-lg {
  --tw-shadow: 0 10px 15px -3px var(--tw-shadow-color, rgb(0 0 0 / 0.1)), 0 4px 6px -4px var(--tw-shadow-color, rgb(0 0 0 / 0.1));
  box-shadow: var(--tw-inset-shadow), var(--tw-inset-ring-shadow), var(--tw-ring-offset-shadow), var(--tw-ring-shadow), var(--tw-shadow);
}
.shadow-md {
  --tw-shadow: 0 4px 6px -1px var(--tw-shadow-color, rgb(0 0 0 / 0.1)), 0 2px 4px -2px var(--tw-shadow-color, rgb(0 0 0 / 0.1));
  box-shadow: var(--tw-inset-shadow), var(--tw-inset-ring-shadow), var(--tw-ring-offset-shadow), var(--tw-ring-shadow), var(--tw-shadow);
}
.shadow-sm {
  --tw-shadow: 0 1px 3px 0 var(--tw-shadow-color, rgb(0 0 0 / 0.1)), 0 1px 2px -1px var(--tw-shadow-color, rgb(0 0 0 / 0.1));
  box-shadow: var(--tw-inset-shadow), var(--tw-inset-ring-shadow), var(--tw-ring-offset-shadow), var(--tw-ring-shadow), var(--tw-shadow);
}
.shadow-xs {
  --tw-shadow: 0 1px 2px 0 var(--tw-shadow-color, rgb(0 0 0 / 0.05));
  box-shadow: var(--tw-inset-shadow), var(--tw-inset-ring-shadow), var(--tw-ring-offset-shadow), var(--tw-ring-shadow), var(--tw-shadow);
}
.ring-0 {
  --tw-ring-shadow: var(--tw-ring-inset,) 0 0 0 calc(0px + var(--tw-ring-offset-width)) var(--tw-ring-color, currentcolor);
  box-shadow: var(--tw-inset-shadow), var(--tw-inset-ring-shadow), var(--tw-ring-offset-shadow), var(--tw-ring-shadow), var(--tw-shadow);
}
.blur {
  --tw-blur: blur(8px);
  filter: var(--tw-blur,) var(--tw-brightness,) var(--tw-contrast,) var(--tw-grayscale,) var(--tw-hue-rotate,) var(--tw-invert,) var(--tw-saturate,) var(--tw-sepia,) var(--tw-drop-shadow,);
}
.backdrop-blur-xl {
  --tw-backdrop-blur: blur(var(--blur-xl));
  -webkit-backdrop-filter: var(--tw-backdrop-blur,) var(--tw-backdrop-brightness,) var(--tw-backdrop-contrast,) var(--tw-backdrop-grayscale,) var(--tw-backdrop-hue-rotate,) var(--tw-backdrop-invert,) var(--tw-backdrop-opacity,) var(--tw-backdrop-saturate,) var(--tw-backdrop-sepia,);
  backdrop-filter: var(--tw-backdrop-blur,) var(--tw-backdrop-brightness,) var(--tw-backdrop-contrast,) var(--tw-backdrop-grayscale,) var(--tw-backdrop-hue-rotate,) var(--tw-backdrop-invert,) var(--tw-backdrop-opacity,) var(--tw-backdrop-saturate,) var(--tw-backdrop-sepia,);
}
.outline-none {
  --tw-outline-style: none;
  outline-style: none;
}
.select-none {
  -webkit-user-select: none;
  user-select: none;
}
.placeholder\\:text-muted-foreground {
  &::placeholder {
    color: var(--t3-muted-foreground);
  }
}
.hover\\:bg-accent {
  &:hover {
    @media (hover: hover) {
      background-color: var(--t3-accent);
    }
  }
}
.hover\\:bg-primary\\/90 {
  &:hover {
    @media (hover: hover) {
      background-color: var(--t3-primary);
      @supports (color: color-mix(in lab, red, red)) {
        background-color: color-mix(in oklab, var(--t3-primary) 90%, transparent);
      }
    }
  }
}
.hover\\:text-accent-foreground {
  &:hover {
    @media (hover: hover) {
      color: var(--t3-accent-foreground);
    }
  }
}
.focus\\:border-b-primary {
  &:focus {
    border-bottom-color: var(--t3-primary);
  }
}
.focus\\:ring-0 {
  &:focus {
    --tw-ring-shadow: var(--tw-ring-inset,) 0 0 0 calc(0px + var(--tw-ring-offset-width)) var(--tw-ring-color, currentcolor);
    box-shadow: var(--tw-inset-shadow), var(--tw-inset-ring-shadow), var(--tw-ring-offset-shadow), var(--tw-ring-shadow), var(--tw-shadow);
  }
}
.focus\\:outline-none {
  &:focus {
    --tw-outline-style: none;
    outline-style: none;
  }
}
.disabled\\:pointer-events-none {
  &:disabled {
    pointer-events: none;
  }
}
.disabled\\:opacity-60 {
  &:disabled {
    opacity: 60%;
  }
}
:host {
  --t3-font-sans: "DM Sans Variable", "DM Sans", -apple-system, BlinkMacSystemFont, "Segoe UI", system-ui,
    sans-serif;
  --t3-font-mono: "SF Mono", "SFMono-Regular", "JetBrains Mono", Consolas, "Liberation Mono", Menlo, monospace;
  --t3-radius: 0.625rem;
  --t3-background: white;
  --t3-foreground: oklch(0.269 0 0);
  --t3-popover: white;
  --t3-popover-foreground: oklch(0.269 0 0);
  --t3-primary: oklch(0.488 0.217 264);
  --t3-primary-foreground: white;
  --t3-muted: rgb(0 0 0 / 4%);
  --t3-muted-foreground: oklch(0.556 0 0);
  --t3-accent: rgb(0 0 0 / 4%);
  --t3-accent-foreground: oklch(0.269 0 0);
  --t3-border: rgb(0 0 0 / 8%);
  --t3-input: rgb(0 0 0 / 10%);
  --t3-ring: oklch(0.488 0.217 264);
  color: var(--t3-foreground);
  font-family: var(--t3-font-sans);
}
* {
  box-sizing: border-box;
  border-color: var(--t3-border);
}
button, input, select, textarea {
  font: inherit;
}
button:focus-visible, input:focus-visible, select:focus-visible, textarea:focus-visible {
  outline: 2px solid var(--t3-ring);
  @supports (color: color-mix(in lab, red, red)) {
    outline: 2px solid color-mix(in srgb, var(--t3-ring) 72%, transparent);
  }
  outline-offset: 1px;
}
@property --tw-translate-x {
  syntax: "*";
  inherits: false;
  initial-value: 0;
}
@property --tw-translate-y {
  syntax: "*";
  inherits: false;
  initial-value: 0;
}
@property --tw-translate-z {
  syntax: "*";
  inherits: false;
  initial-value: 0;
}
@property --tw-border-style {
  syntax: "*";
  inherits: false;
  initial-value: solid;
}
@property --tw-leading {
  syntax: "*";
  inherits: false;
}
@property --tw-font-weight {
  syntax: "*";
  inherits: false;
}
@property --tw-shadow {
  syntax: "*";
  inherits: false;
  initial-value: 0 0 #0000;
}
@property --tw-shadow-color {
  syntax: "*";
  inherits: false;
}
@property --tw-shadow-alpha {
  syntax: "<percentage>";
  inherits: false;
  initial-value: 100%;
}
@property --tw-inset-shadow {
  syntax: "*";
  inherits: false;
  initial-value: 0 0 #0000;
}
@property --tw-inset-shadow-color {
  syntax: "*";
  inherits: false;
}
@property --tw-inset-shadow-alpha {
  syntax: "<percentage>";
  inherits: false;
  initial-value: 100%;
}
@property --tw-ring-color {
  syntax: "*";
  inherits: false;
}
@property --tw-ring-shadow {
  syntax: "*";
  inherits: false;
  initial-value: 0 0 #0000;
}
@property --tw-inset-ring-color {
  syntax: "*";
  inherits: false;
}
@property --tw-inset-ring-shadow {
  syntax: "*";
  inherits: false;
  initial-value: 0 0 #0000;
}
@property --tw-ring-inset {
  syntax: "*";
  inherits: false;
}
@property --tw-ring-offset-width {
  syntax: "<length>";
  inherits: false;
  initial-value: 0px;
}
@property --tw-ring-offset-color {
  syntax: "*";
  inherits: false;
  initial-value: #fff;
}
@property --tw-ring-offset-shadow {
  syntax: "*";
  inherits: false;
  initial-value: 0 0 #0000;
}
@property --tw-blur {
  syntax: "*";
  inherits: false;
}
@property --tw-brightness {
  syntax: "*";
  inherits: false;
}
@property --tw-contrast {
  syntax: "*";
  inherits: false;
}
@property --tw-grayscale {
  syntax: "*";
  inherits: false;
}
@property --tw-hue-rotate {
  syntax: "*";
  inherits: false;
}
@property --tw-invert {
  syntax: "*";
  inherits: false;
}
@property --tw-opacity {
  syntax: "*";
  inherits: false;
}
@property --tw-saturate {
  syntax: "*";
  inherits: false;
}
@property --tw-sepia {
  syntax: "*";
  inherits: false;
}
@property --tw-drop-shadow {
  syntax: "*";
  inherits: false;
}
@property --tw-drop-shadow-color {
  syntax: "*";
  inherits: false;
}
@property --tw-drop-shadow-alpha {
  syntax: "<percentage>";
  inherits: false;
  initial-value: 100%;
}
@property --tw-drop-shadow-size {
  syntax: "*";
  inherits: false;
}
@property --tw-backdrop-blur {
  syntax: "*";
  inherits: false;
}
@property --tw-backdrop-brightness {
  syntax: "*";
  inherits: false;
}
@property --tw-backdrop-contrast {
  syntax: "*";
  inherits: false;
}
@property --tw-backdrop-grayscale {
  syntax: "*";
  inherits: false;
}
@property --tw-backdrop-hue-rotate {
  syntax: "*";
  inherits: false;
}
@property --tw-backdrop-invert {
  syntax: "*";
  inherits: false;
}
@property --tw-backdrop-opacity {
  syntax: "*";
  inherits: false;
}
@property --tw-backdrop-saturate {
  syntax: "*";
  inherits: false;
}
@property --tw-backdrop-sepia {
  syntax: "*";
  inherits: false;
}
@layer properties {
  @supports ((-webkit-hyphens: none) and (not (margin-trim: inline))) or ((-moz-orient: inline) and (not (color:rgb(from red r g b)))) {
    *, ::before, ::after, ::backdrop {
      --tw-translate-x: 0;
      --tw-translate-y: 0;
      --tw-translate-z: 0;
      --tw-border-style: solid;
      --tw-leading: initial;
      --tw-font-weight: initial;
      --tw-shadow: 0 0 #0000;
      --tw-shadow-color: initial;
      --tw-shadow-alpha: 100%;
      --tw-inset-shadow: 0 0 #0000;
      --tw-inset-shadow-color: initial;
      --tw-inset-shadow-alpha: 100%;
      --tw-ring-color: initial;
      --tw-ring-shadow: 0 0 #0000;
      --tw-inset-ring-color: initial;
      --tw-inset-ring-shadow: 0 0 #0000;
      --tw-ring-inset: initial;
      --tw-ring-offset-width: 0px;
      --tw-ring-offset-color: #fff;
      --tw-ring-offset-shadow: 0 0 #0000;
      --tw-blur: initial;
      --tw-brightness: initial;
      --tw-contrast: initial;
      --tw-grayscale: initial;
      --tw-hue-rotate: initial;
      --tw-invert: initial;
      --tw-opacity: initial;
      --tw-saturate: initial;
      --tw-sepia: initial;
      --tw-drop-shadow: initial;
      --tw-drop-shadow-color: initial;
      --tw-drop-shadow-alpha: 100%;
      --tw-drop-shadow-size: initial;
      --tw-backdrop-blur: initial;
      --tw-backdrop-brightness: initial;
      --tw-backdrop-contrast: initial;
      --tw-backdrop-grayscale: initial;
      --tw-backdrop-hue-rotate: initial;
      --tw-backdrop-invert: initial;
      --tw-backdrop-opacity: initial;
      --tw-backdrop-saturate: initial;
      --tw-backdrop-sepia: initial;
    }
  }
}
`,Z=`data-t3code-annotation-ui`,Fr=2147483646,Ir=`var(--t3-primary)`,Lr=`color-mix(in srgb, var(--t3-primary) 10%, transparent)`,Rr=20,zr=1,Br=10,Vr=null,Hr=0,Ur=null,Q={send:()=>{},on:()=>{},off:()=>{}},Wr=(e,t)=>{if(!t)return;e.style.colorScheme=t.colorScheme;let n={"--t3-radius":t.radius,"--t3-background":t.background,"--t3-foreground":t.foreground,"--t3-popover":t.popover,"--t3-popover-foreground":t.popoverForeground,"--t3-primary":t.primary,"--t3-primary-foreground":t.primaryForeground,"--t3-muted":t.muted,"--t3-muted-foreground":t.mutedForeground,"--t3-accent":t.accent,"--t3-accent-foreground":t.accentForeground,"--t3-border":t.border,"--t3-input":t.input,"--t3-ring":t.ring,"--t3-font-sans":t.fontSans,"--t3-font-mono":t.fontMono};for(let[t,r]of Object.entries(n))e.style.setProperty(t,r)},Gr=e=>{e.isTrusted&&Q.send(o,{kind:`pointer`,x:e.clientX,y:e.clientY,button:e.button})},Kr=e=>{e.isTrusted&&Q.send(o,{kind:`key`,key:e.key,code:e.code})},qr=e=>(Hr+=1,`${e}_${Hr.toString(36)}`),Jr=e=>({x:e.left,y:e.top,width:e.width,height:e.height}),Yr=(e,t,n,r)=>({x:Math.min(e,n),y:Math.min(t,r),width:Math.abs(n-e),height:Math.abs(r-t)}),Xr=e=>e.width>=3&&e.height>=3;function Zr(e,t=20){if(e.length===0)return null;let n=Math.min(...e.map(e=>e.x)),r=Math.min(...e.map(e=>e.y)),i=Math.max(...e.map(e=>e.x+e.width)),a=Math.max(...e.map(e=>e.y+e.height)),o=Math.max(0,n-t),s=Math.max(0,r-t),c=Math.max(1,window.innerWidth-o),l=Math.max(1,window.innerHeight-s);return{x:o,y:s,width:Math.min(c,i-n+t*2),height:Math.min(l,a-r+t*2)}}function Qr(e){return e instanceof Element&&e.closest(`[${Z}]`)!==null}function $r(e,t){for(let n of document.elementsFromPoint(e,t))if(n instanceof Element&&!Qr(n)&&!(n===document.documentElement||n===document.body))return n;return null}function ei(e){return`${e.tagName.toLowerCase()}${e.id?`#${e.id}`:``}${e instanceof HTMLElement&&typeof e.className==`string`?e.className.trim().split(/\s+/).filter(Boolean).slice(0,2).map(e=>`.${e}`).join(``):``}`}function ti(e,t){let n=document.createElement(`div`);return n.setAttribute(Z,``),n.style.cssText=[`position:fixed`,`pointer-events:none`,`border:2px solid ${e}`,`background:${t}`,`border-radius:3px`,`box-sizing:border-box`,`display:none`,`z-index:${zr}`].join(`;`),n}function ni(e,t){e.style.display=`block`,e.style.transform=`translate(${t.x}px, ${t.y}px)`,e.style.width=`${t.width}px`,e.style.height=`${t.height}px`}function ri(){let e=document.createElement(`div`);return e.setAttribute(Z,``),e.className=`fixed z-1 max-w-70 overflow-hidden rounded-md bg-primary px-2 py-1 font-sans text-xs font-semibold text-primary-foreground shadow-md`,e.style.cssText=[`position:fixed`,`pointer-events:none`,`white-space:nowrap`,`text-overflow:ellipsis`,`z-index:${zr}`].join(`;`),e}function ii(e){if(!e.element.isConnected){e.outline.style.display=`none`,e.label.style.display=`none`;return}let t=e.element.getBoundingClientRect();ni(e.outline,Jr(t)),e.label.textContent=ei(e.element),e.label.style.display=`block`,e.label.style.transform=`translate(${Math.max(4,t.left)}px, ${Math.max(4,t.top-22)}px)`}function ai(e){return{functionName:e.functionName??null,fileName:e.fileName??null,lineNumber:e.lineNumber??null,columnNumber:e.columnNumber??null}}async function oi(e){try{let t=await Nr(e),n=(t.stack??[]).map(ai);return{pageUrl:location.href,pageTitle:document.title?.trim()||null,tagName:e.tagName.toLowerCase(),selector:t.selector,htmlPreview:t.htmlPreview??``,componentName:t.componentName,source:n[0]??null,stack:n,styles:t.styles??``,pickedAt:new Date().toISOString()}}catch{return null}}function si(e,t){let n=document.createElement(`button`);return n.type=`button`,n.textContent=e,n.title=t,n.className=`inline-flex h-7 cursor-pointer items-center justify-center rounded-md border border-transparent px-2 font-sans text-xs font-medium text-foreground outline-none hover:bg-accent disabled:pointer-events-none disabled:opacity-60`,n}function ci(e){e.setAttribute(`aria-label`,e.getAttribute(`aria-label`)??`Style value`),e.className=`h-7 min-w-0 w-full appearance-none rounded-md border border-input bg-background px-2 font-mono text-xs text-foreground shadow-xs outline-none`}function li(e){let t=document.createElement(`div`);t.style.cssText=`position:relative;min-width:0`;let n=document.createElement(`span`);return n.textContent=e.dataset.unit??``,n.className=`pointer-events-none absolute top-1/2 right-2 -translate-y-1/2 font-mono text-xs text-muted-foreground`,t.append(e,n),t}function $(e,t){let n=document.createElement(`label`);n.className=`grid min-h-7 grid-cols-[82px_minmax(0,1fr)] items-center gap-2 font-sans text-xs font-medium text-muted-foreground`;let r=document.createElement(`span`);return r.textContent=e,ci(t),n.append(r,t instanceof HTMLInputElement&&t.dataset.unit?li(t):t),n}function ui(){let e=document.createElement(`section`);return e.className=`grid gap-1 border-t border-border py-2`,e}function di(e,t=`0`){let n=document.createElement(`input`);return n.type=`number`,n.placeholder=t,n.style.paddingRight=`30px`,n.dataset.unit=e,n}function fi(e){if(e.length===0)return``;if(e.length===1)return`M ${e[0].x} ${e[0].y} l 0.01 0.01`;let t=`M ${e[0].x} ${e[0].y}`;for(let n=1;n<e.length-1;n+=1){let r=e[n],i=e[n+1];t+=` Q ${r.x} ${r.y} ${(r.x+i.x)/2} ${(r.y+i.y)/2}`}let n=e[e.length-1];return t+=` L ${n.x} ${n.y}`,t}function pi(e,t){let n=e.map(e=>e.x),r=e.map(e=>e.y),i=t+3,a=Math.min(...n)-i,o=Math.min(...r)-i,s=Math.max(...n)+i,c=Math.max(...r)+i;return{x:a,y:o,width:s-a,height:c-o}}function mi(){Vr?.teardown(!1);let e=!1,t=document.createElement(`div`);t.setAttribute(Z,``),t.style.cssText=`position:fixed;inset:0;z-index:${Fr};pointer-events:none`,Wr(t,Ur);let a=t.attachShadow({mode:`closed`}),o=document.createElement(`style`);o.textContent=Pr,a.appendChild(o);let s=document.createElement(`div`);s.setAttribute(Z,``),s.className=`fixed inset-0 font-sans text-foreground`,s.style.cssText=`pointer-events:none`;let c=document.createElement(`style`);c.setAttribute(Z,``),c.textContent=`html[data-t3code-annotation-tool] body, html[data-t3code-annotation-tool] body * { cursor: crosshair !important; } [${Z}], [${Z}] * { cursor: default !important; } [${Z}] input[type=number]::-webkit-inner-spin-button, [${Z}] input[type=number]::-webkit-outer-spin-button { appearance:none; margin:0; }`,document.documentElement.appendChild(c),a.appendChild(s);let l=ti(Ir,Lr),u=ti(Ir,Lr);s.append(l,u);let d=document.createElementNS(`http://www.w3.org/2000/svg`,`svg`);d.setAttribute(Z,``),d.setAttribute(`width`,`100%`),d.setAttribute(`height`,`100%`),d.setAttribute(`viewBox`,`0 0 ${window.innerWidth} ${window.innerHeight}`),d.style.cssText=`position:fixed;inset:0;overflow:visible;pointer-events:none`,d.style.zIndex=String(zr),s.appendChild(d);let f=document.createElement(`div`);f.setAttribute(Z,``),f.className=`pointer-events-auto fixed top-2.5 left-1/2 flex -translate-x-1/2 gap-0.5 rounded-lg border border-border bg-popover/95 p-1 text-popover-foreground shadow-lg backdrop-blur-xl`,f.style.zIndex=String(Br),s.appendChild(f);let p=document.createElement(`div`);p.setAttribute(Z,``),p.className=`pointer-events-auto fixed hidden max-h-[calc(100vh-16px)] w-[min(360px,calc(100vw-16px))] flex-col overflow-hidden rounded-xl border border-border bg-popover/96 text-popover-foreground shadow-2xl backdrop-blur-xl`,p.style.zIndex=String(Br),s.appendChild(p);let m=document.createElement(`div`);m.className=`flex items-start gap-2 p-2`;let h=si(``,`Expand annotation editor`);h.setAttribute(`aria-label`,`Expand annotation editor`),h.setAttribute(`aria-expanded`,`false`),h.className+=` h-8 w-8 shrink-0 bg-muted p-0 text-muted-foreground hover:bg-accent hover:text-accent-foreground`,h.innerHTML=`<svg viewBox="0 0 20 20" width="15" height="15" aria-hidden="true"><path d="M4 5h12M4 10h12M4 15h12M7 3v4M13 8v4M9 13v4" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round"/></svg>`,m.appendChild(h);let g=document.createElement(`textarea`);g.placeholder=`Describe the change…`,g.rows=1,g.className=`min-h-8 max-h-24 min-w-0 flex-1 resize-none overflow-y-hidden border-0 border-b border-b-transparent bg-transparent px-0 py-1.5 font-sans text-sm leading-5 text-foreground outline-none ring-0 placeholder:text-muted-foreground focus:border-b-primary focus:outline-none focus:ring-0`,m.appendChild(g);let _=document.createElement(`button`);_.type=`button`,_.textContent=`⠿`,_.title=`Drag annotation editor`,_.className=`hidden h-8 w-6 shrink-0 cursor-grab select-none border-0 bg-transparent p-0 font-sans text-lg font-bold leading-5 text-muted-foreground`,m.appendChild(_);let v=si(`Attach`,`Attach annotation and screenshot`);v.className+=` h-8 shrink-0 border-primary bg-primary px-3 text-primary-foreground shadow-sm hover:bg-primary/90`,m.appendChild(v),p.appendChild(m);let y=document.createElement(`div`);y.className=`hidden max-h-[min(176px,calc(100vh-180px))] overflow-auto border-t border-border bg-muted/40 px-3`,p.appendChild(y);let b=new Map,x=[],S=[],ee=new Map,C=new Map,w=`select`,T=null,E=null,te=!1,D=!1,ne=!1,O=null,k=null,A=null;g.addEventListener(`input`,()=>{g.style.height=`auto`;let e=Math.min(g.scrollHeight,96);g.style.height=`${e}px`,g.style.overflowY=g.scrollHeight>96?`auto`:`hidden`,Oe()});let j=()=>{let e=b.size>0||x.length>0||S.length>0;p.style.display=e?`flex`:`none`,v.disabled=!e,v.style.opacity=e?`1`:`0.45`,h.disabled=!e,y.style.display=D&&b.size>0?`grid`:`none`,Oe(),e&&!ne&&(ne=!0,window.setTimeout(()=>g.focus({preventScroll:!0}),0))},re=()=>{for(let[e,t]of C){let n=e===w;t.classList.toggle(`bg-primary/10`,n),t.classList.toggle(`text-primary`,n),t.classList.toggle(`text-foreground`,!n)}w!==`select`&&(l.style.display=`none`),w!==`marquee`&&(u.style.display=`none`),document.documentElement.setAttribute(`data-t3code-annotation-tool`,w)},ie=e=>{if(e.element instanceof HTMLElement||e.element instanceof SVGElement)for(let[t,n]of e.baselineStyles)n?e.element.style.setProperty(t,n):e.element.style.removeProperty(t);b.delete(e.element),e.outline.remove(),e.label.remove();for(let[t,n]of ee)n.targetId===e.id&&ee.delete(t);j()},ae=e=>{if(b.has(e))return;let t={id:qr(`element`),element:e,outline:ti(Ir,Lr),label:ri(),baselineStyles:new Map};b.set(e,t),s.append(t.outline,t.label),ii(t),j(),D&&(y.style.display=`grid`,Ce())},oe=(e,t)=>{let n=b.get(e);if(n){ie(n);return}if(!t)for(let e of Array.from(b.values()))ie(e);ae(e)},M=(e,t)=>{for(let n of b.values()){if(!(n.element instanceof HTMLElement||n.element instanceof SVGElement))continue;n.baselineStyles.has(e)||n.baselineStyles.set(e,n.element.style.getPropertyValue(e));let r=`${n.id}:${e}`,i=ee.get(r)?.previousValue??getComputedStyle(n.element).getPropertyValue(e).trim();n.element.style.setProperty(e,t,`important`),ee.set(r,{targetId:n.id,selector:null,property:e,previousValue:i,value:t}),ii(n)}},se=ui(),ce=ui(),le=ui(),N=ui();y.append(se,ce,le,N);let P=document.createElement(`select`);for(let e of[`inherit`,`system-ui`,`sans-serif`,`serif`,`monospace`]){let t=document.createElement(`option`);t.value=e,t.textContent=e,P.appendChild(t)}P.addEventListener(`change`,()=>M(`font-family`,P.value)),se.appendChild($(`Font`,P));let F=di(`px`,`16`);F.min=`1`,F.max=`300`,F.addEventListener(`input`,()=>{F.value&&M(`font-size`,`${F.value}px`)}),se.appendChild($(`Font size`,F));let ue=document.createElement(`select`);for(let e of[`300`,`400`,`500`,`600`,`700`,`800`,`900`]){let t=document.createElement(`option`);t.value=e,t.textContent=e,ue.appendChild(t)}ue.addEventListener(`change`,()=>M(`font-weight`,ue.value)),se.appendChild($(`Font weight`,ue));let I=document.createElement(`input`);I.type=`text`,I.placeholder=`normal / 1.4`,I.addEventListener(`change`,()=>{I.value.trim()&&M(`line-height`,I.value.trim())}),se.appendChild($(`Line height`,I));let de=(e,t,n)=>{let r=document.createElement(`label`);r.className=`grid min-h-7 grid-cols-[82px_minmax(0,1fr)] items-center gap-2 font-sans text-xs font-medium text-muted-foreground`;let i=document.createElement(`span`);i.textContent=e;let a=document.createElement(`div`);a.className=`grid h-7 grid-cols-[22px_minmax(0,1fr)] items-center gap-1 rounded-md border border-input bg-background px-1 shadow-xs`;let o=document.createElement(`input`);o.type=`color`,o.setAttribute(`aria-label`,e),o.style.cssText=`width:20px;height:20px;padding:0;border:0;border-radius:5px;overflow:hidden;background:transparent;cursor:pointer`;let s=document.createElement(`input`);return s.type=`text`,s.setAttribute(`aria-label`,`${e} value`),s.className=`min-w-0 w-full border-0 bg-transparent font-mono text-xs text-foreground outline-none`,o.addEventListener(`input`,()=>{s.value=o.value,M(t,o.value)}),s.addEventListener(`change`,()=>{let e=s.value.trim();e&&(M(t,e),/^#[0-9a-f]{6}$/i.test(e)&&(o.value=e))}),a.append(o,s),r.append(i,a),n.appendChild(r),{row:r,color:o,text:s}},fe=de(`Text color`,`color`,ce),pe=de(`Background`,`background-color`,ce),L=document.createElement(`input`);L.type=`range`,L.min=`0`,L.max=`1`,L.step=`0.05`,L.value=`1`,L.style.accentColor=Ir,L.addEventListener(`input`,()=>M(`opacity`,L.value)),ce.appendChild($(`Opacity`,L));let R=di(`px`,`0`);R.min=`0`,R.max=`300`,R.addEventListener(`input`,()=>{R.value&&M(`border-radius`,`${R.value}px`)}),le.appendChild($(`Radius`,R));let me=de(`Border color`,`border-color`,le),z=di(`px`,`0`);z.min=`0`,z.max=`100`,z.addEventListener(`input`,()=>{z.value&&(M(`border-style`,`solid`),M(`border-width`,`${z.value}px`))}),le.appendChild($(`Border width`,z));let he=document.createElement(`div`);he.style.cssText=`display:grid;grid-template-columns:82px minmax(0,1fr);gap:8px;align-items:center`;let ge=document.createElement(`div`);ge.className=`grid gap-2 font-sans text-xs font-medium text-muted-foreground`,ge.innerHTML=`<span>Width</span><span>Height</span>`;let _e=document.createElement(`div`);_e.style.cssText=`position:relative;display:grid;gap:3px;padding-left:22px`;let B=di(`px`,`auto`),V=di(`px`,`auto`);ci(B),ci(V);let H=si(``,`Lock aspect ratio`);H.setAttribute(`aria-pressed`,`true`),H.style.cssText+=`;position:absolute;left:0;top:50%;transform:translateY(-50%);width:18px;height:38px;padding:0`,H.className+=` bg-primary/10 text-primary`,_e.append(li(B),li(V),H),he.append(ge,_e),N.appendChild(he);let U=!0,W=1,ve=()=>{H.innerHTML=U?`<svg viewBox="0 0 20 20" width="14" height="14" aria-hidden="true"><path d="M8 6.5 9.5 5A3.5 3.5 0 0 1 14.5 10l-1.5 1.5M12 13.5 10.5 15A3.5 3.5 0 0 1 5.5 10L7 8.5M7.5 12.5l5-5" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round"/></svg>`:`<svg viewBox="0 0 20 20" width="14" height="14" aria-hidden="true"><path d="m6 6 8 8M8 6.5 9.5 5A3.5 3.5 0 0 1 14 9M12 13.5 10.5 15A3.5 3.5 0 0 1 6 11" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round"/></svg>`,H.setAttribute(`aria-pressed`,String(U)),H.classList.toggle(`bg-primary/10`,U),H.classList.toggle(`text-primary`,U),H.classList.toggle(`bg-muted`,!U),H.classList.toggle(`text-muted-foreground`,!U)};H.addEventListener(`click`,()=>{U=!U,ve()}),B.addEventListener(`input`,()=>{let e=Number(B.value);if(!(!Number.isFinite(e)||e<=0)&&(M(`width`,`${e}px`),U&&W>0)){let t=Math.max(1,Math.round(e/W));V.value=String(t),M(`height`,`${t}px`)}}),V.addEventListener(`input`,()=>{let e=Number(V.value);if(!(!Number.isFinite(e)||e<=0)&&(M(`height`,`${e}px`),U&&W>0)){let t=Math.max(1,Math.round(e*W));B.value=String(t),M(`width`,`${t}px`)}}),ve();let ye=(e,t,n)=>{let r=document.createElement(`input`);return r.type=`text`,r.placeholder=n,r.addEventListener(`change`,()=>{r.value.trim()&&M(t,r.value.trim())}),N.appendChild($(e,r)),r},be=ye(`Padding`,`padding`,`0 0 0 0`),xe=ye(`Margin`,`margin`,`0 0 0 0`),Se=ye(`Gap`,`gap`,`0px`),Ce=()=>{let e=b.values().next().value;if(!e)return;let t=getComputedStyle(e.element),n=e.element.getBoundingClientRect();W=n.height>0?n.width/n.height:1,B.value=String(Math.round(n.width)),V.value=String(Math.round(n.height)),F.value=String(Math.round(Number.parseFloat(t.fontSize)||16)),ue.value=t.fontWeight.match(/^[0-9]+$/)?t.fontWeight:`400`,I.value=t.lineHeight,P.value=Array.from(P.options).some(e=>e.value===t.fontFamily)?t.fontFamily:`inherit`,fe.text.value=t.color,pe.text.value=t.backgroundColor,me.text.value=t.borderColor,L.value=t.opacity,R.value=String(Math.round(Number.parseFloat(t.borderRadius)||0)),z.value=String(Math.round(Number.parseFloat(t.borderWidth)||0)),be.value=t.padding,xe.value=t.margin,Se.value=t.gap===`normal`?`0px`:t.gap};for(let[e,t,n]of[[`select`,`Select`,`Select elements (V)`],[`marquee`,`Region`,`Draw a region or marquee-select elements (R)`],[`draw`,`Draw`,`Draw freehand (D)`],[`erase`,`Erase`,`Remove an annotation target (E)`]]){let r=si(t,n);r.className+=` h-8 px-2.5 text-sm`,r.addEventListener(`click`,()=>{w=e,re()}),C.set(e,r),f.appendChild(r)}let we=(e,t)=>{let n=p.getBoundingClientRect();return{left:Math.min(Math.max(8,e),Math.max(8,window.innerWidth-n.width-8)),top:Math.min(Math.max(8,t),Math.max(8,window.innerHeight-n.height-8))}},Te=e=>{let t=we(e.left,e.top);p.style.left=`${t.left}px`,p.style.top=`${t.top}px`,p.style.right=`auto`,p.style.bottom=`auto`,D&&(O=t)},Ee=()=>Zr([...Array.from(b.values(),e=>Jr(e.element.getBoundingClientRect())),...x.map(e=>e.rect),...S.map(e=>e.bounds)],0),De=()=>{let e=Ee();if(!e)return;let t=p.getBoundingClientRect(),n=[{left:e.x+e.width+8,top:e.y},{left:e.x-t.width-8,top:e.y},{left:e.x+e.width-t.width,top:e.y+e.height+8},{left:e.x+e.width-t.width,top:e.y-t.height-8}],r=e=>Math.max(0,-e.left)+Math.max(0,-e.top)+Math.max(0,e.left+t.width-window.innerWidth)+Math.max(0,e.top+t.height-window.innerHeight),i=n.reduce((e,t)=>r(t)<r(e)?t:e);Te(i)};function Oe(){A!==null&&window.cancelAnimationFrame(A),A=window.requestAnimationFrame(()=>{A=null,p.style.display!==`none`&&(D&&O?Te(O):De())})}h.addEventListener(`click`,()=>{if(b.size!==0){if(D)D=!1,O=null,y.style.display=`none`,_.style.display=`none`,h.setAttribute(`aria-expanded`,`false`),h.title=`Expand annotation editor`,h.setAttribute(`aria-label`,`Expand annotation editor`);else{let e=p.getBoundingClientRect();D=!0,O={left:e.left,top:e.top},y.style.display=b.size>0?`grid`:`none`,_.style.display=`block`,h.setAttribute(`aria-expanded`,`true`),h.title=`Collapse annotation editor`,h.setAttribute(`aria-label`,`Collapse annotation editor`),b.size>0&&Ce()}Oe()}});let ke=e=>{if(e.button!==0||!D)return;let t=p.getBoundingClientRect();k={pointerId:e.pointerId,offsetX:e.clientX-t.left,offsetY:e.clientY-t.top},_.setPointerCapture(e.pointerId),_.style.cursor=`grabbing`,e.preventDefault(),e.stopPropagation()},Ae=e=>{!k||k.pointerId!==e.pointerId||(Te({left:e.clientX-k.offsetX,top:e.clientY-k.offsetY}),e.preventDefault(),e.stopPropagation())},G=e=>{!k||k.pointerId!==e.pointerId||(k=null,_.style.cursor=`grab`,_.hasPointerCapture(e.pointerId)&&_.releasePointerCapture(e.pointerId),e.preventDefault(),e.stopPropagation())};_.addEventListener(`pointerdown`,ke),_.addEventListener(`pointermove`,Ae),_.addEventListener(`pointerup`,G),_.addEventListener(`pointercancel`,G);let K=()=>{for(let e of b.values())ii(e);Oe()},je=(e,t)=>{for(let n of Array.from(b.values()).toReversed()){let r=n.element.getBoundingClientRect();if(e>=r.left&&e<=r.right&&t>=r.top&&t<=r.bottom)return ie(n),!0}let n=x.findIndex(n=>e>=n.rect.x&&e<=n.rect.x+n.rect.width&&t>=n.rect.y&&t<=n.rect.y+n.rect.height);if(n>=0){let[e]=x.splice(n,1);return s.querySelector(`[data-region-id="${e?.id}"]`)?.remove(),j(),!0}let r=S.findIndex(n=>e>=n.bounds.x&&e<=n.bounds.x+n.bounds.width&&t>=n.bounds.y&&t<=n.bounds.y+n.bounds.height);if(r>=0){let[e]=S.splice(r,1);return d.querySelector(`[data-stroke-id="${e?.id}"]`)?.remove(),j(),!0}return!1},Me=e=>{let t=Array.from(document.querySelectorAll(`body *`)).filter(e=>!Qr(e)).map(e=>({element:e,rect:e.getBoundingClientRect()})).filter(({rect:t})=>t.width<2||t.height<2?!1:!(t.right<e.x||t.left>e.x+e.width||t.bottom<e.y||t.top>e.y+e.height)).filter(({element:t,rect:n})=>{let r=n.left+n.width/2,i=n.top+n.height/2;return r>=e.x&&r<=e.x+e.width&&i>=e.y&&i<=e.y+e.height&&(t.children.length===0||t instanceof HTMLButtonElement||t instanceof HTMLAnchorElement||t.getAttribute(`role`)===`button`)}).sort((e,t)=>e.rect.width*e.rect.height-t.rect.width*t.rect.height).slice(0,Rr);for(let e of t)ae(e.element);return t.length},Ne=()=>{l.style.display=`none`},Pe=e=>{if(Qr(e.target)){Ne();return}if(w===`select`&&T===null){let t=$r(e.clientX,e.clientY);t?ni(l,Jr(t.getBoundingClientRect())):Ne();return}if(Ne(),w===`marquee`&&T){ni(u,Yr(T.x,T.y,e.clientX,e.clientY));return}w===`draw`&&E&&(E.target.points=[...E.target.points,{x:e.clientX,y:e.clientY}],E.target.bounds=pi(E.target.points,E.target.width),E.path.setAttribute(`d`,fi(E.target.points)))},Fe=e=>{if(!(e.button!==0||Qr(e.target))){if(e.preventDefault(),e.stopPropagation(),w===`select`){let t=$r(e.clientX,e.clientY);t&&oe(t,e.shiftKey);return}if(w===`erase`){je(e.clientX,e.clientY);return}if(T={x:e.clientX,y:e.clientY},w===`draw`){let e={id:qr(`stroke`),color:Ur?.primary??`#2563eb`,width:4,points:[T],bounds:{x:T.x,y:T.y,width:1,height:1}},t=document.createElementNS(`http://www.w3.org/2000/svg`,`path`);t.setAttribute(Z,``),t.setAttribute(`data-stroke-id`,e.id),t.setAttribute(`fill`,`none`),t.setAttribute(`stroke`,e.color),t.setAttribute(`stroke-width`,String(e.width)),t.setAttribute(`stroke-linecap`,`round`),t.setAttribute(`stroke-linejoin`,`round`),d.appendChild(t),E={target:e,path:t}}}},Ie=e=>{if(T){if(e.preventDefault(),e.stopPropagation(),w===`marquee`){let t=Yr(T.x,T.y,e.clientX,e.clientY);if(u.style.display=`none`,Xr(t)&&Me(t)===0){let e={id:qr(`region`),rect:t};x.push(e);let n=ti(Ir,`color-mix(in srgb, var(--t3-primary) 6%, transparent)`);n.setAttribute(`data-region-id`,e.id),ni(n,t),s.appendChild(n)}}else w===`draw`&&E&&(E.target.points.length>1?S.push(E.target):E.path.remove(),E=null);T=null,j()}},Le=e=>{Qr(e.target)||(e.preventDefault(),e.stopPropagation())},Re=e=>{e.relatedTarget===null&&Ne()},ze=()=>{Ne()},Be=()=>{for(let e of b.values())if(e.element instanceof HTMLElement||e.element instanceof SVGElement)for(let[t,n]of e.baselineStyles)n?e.element.style.setProperty(t,n):e.element.style.removeProperty(t)},Ve=a=>{e||(e=!0,Be(),window.removeEventListener(`pointermove`,Pe,!0),window.removeEventListener(`pointerdown`,Fe,!0),window.removeEventListener(`pointerup`,Ie,!0),window.removeEventListener(`pointerout`,Re,!0),window.removeEventListener(`click`,Le,!0),window.removeEventListener(`blur`,ze),window.removeEventListener(`keydown`,Ue,!0),window.removeEventListener(`scroll`,K,!0),window.removeEventListener(`resize`,K),_.removeEventListener(`pointerdown`,ke),_.removeEventListener(`pointermove`,Ae),_.removeEventListener(`pointerup`,G),_.removeEventListener(`pointercancel`,G),A!==null&&window.cancelAnimationFrame(A),Q.off(n,He),Q.off(i,q),document.documentElement.removeAttribute(`data-t3code-annotation-tool`),c.remove(),t.remove(),Vr=null,a&&Q.send(r,null))},He=()=>Ve(!1),q=()=>Ve(!1),Ue=e=>{if(!(Qr(e.target)&&e.key!==`Escape`)){if(e.key===`Escape`){e.preventDefault(),e.stopPropagation(),Ve(!0);return}if(e.key===`v`)w=`select`;else if(e.key===`r`)w=`marquee`;else if(e.key===`d`)w=`draw`;else if(e.key===`e`)w=`erase`;else return;re()}};v.addEventListener(`click`,()=>{te||b.size===0&&x.length===0&&S.length===0||(te=!0,v.disabled=!0,v.textContent=`Capturing…`,Promise.all(Array.from(b.values()).map(async e=>{let t=await oi(e.element);if(!t)return null;for(let n of ee.values())n.targetId===e.id&&(n.selector=t.selector);return{id:e.id,element:t,rect:Jr(e.element.getBoundingClientRect())}})).then(e=>{let t=e.filter(e=>e!==null),n={id:qr(`annotation`),pageUrl:location.href,pageTitle:document.title?.trim()||null,comment:g.value.trim(),elements:t,regions:[...x],strokes:[...S],styleChanges:Array.from(ee.values()),screenshot:null,createdAt:new Date().toISOString()};p.style.display=`none`,f.style.display=`none`,l.style.display=`none`;let i=Zr([...t.map(e=>e.rect),...x.map(e=>e.rect),...S.map(e=>e.bounds)]);Q.send(r,n,i)}))}),g.addEventListener(`keydown`,e=>{e.key!==`Enter`||!(e.metaKey||e.ctrlKey)||(e.preventDefault(),v.click())}),window.addEventListener(`pointermove`,Pe,{capture:!0,passive:!1}),window.addEventListener(`pointerdown`,Fe,{capture:!0,passive:!1}),window.addEventListener(`pointerup`,Ie,{capture:!0,passive:!1}),window.addEventListener(`pointerout`,Re,{capture:!0,passive:!0}),window.addEventListener(`click`,Le,{capture:!0,passive:!1}),window.addEventListener(`blur`,ze),window.addEventListener(`keydown`,Ue,{capture:!0}),window.addEventListener(`scroll`,K,{capture:!0,passive:!0}),window.addEventListener(`resize`,K,{passive:!0}),Q.on(n,He),Q.on(i,q),document.documentElement.appendChild(t),re(),j(),Vr={teardown:Ve,applyTheme:e=>Wr(t,e)}}function hi(e){return typeof e==`object`&&e?e:null}function gi(e){Q=e,window.addEventListener(`pointerdown`,Gr,!0),window.addEventListener(`keydown`,Kr,!0),e.on(t,e=>{let t=hi(e);t&&(Ur=t),mi()}),e.on(a,e=>{let t=hi(e);t&&(Ur=t,Vr?.applyTheme(t))}),e.on(n,()=>Vr?.teardown(!1)),e.on(c,()=>e.send(s)),e.send(s)}function _i(e){let t=new Map,n=`*`;return window.addEventListener(`message`,r=>{if(r.source!==e)return;let i=u(r.data);if(i!==null){n=r.origin;for(let e of t.get(i.channel)??[])e(...i.args)}}),{send:(t,...r)=>{n===`*`&&t!==`preview:annotation-ready`||e.postMessage(d(t,r),n)},on:(e,n)=>{let r=t.get(e);r?r.add(n):t.set(e,new Set([n]))},off:(e,n)=>{t.get(e)?.delete(n)}}}function vi(){return typeof window>`u`||window.parent===window?!1:(gi(_i(window.parent)),!0)}return vi(),e.startPreviewAnnotation=vi,e})({});