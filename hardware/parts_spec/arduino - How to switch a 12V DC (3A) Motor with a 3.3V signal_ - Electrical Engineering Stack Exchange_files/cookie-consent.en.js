(()=>{"use strict";var t={p:""};t.p=document.getElementById("webpack-public-path").innerText+"Js/",(()=>{function t(){}var e=function(t,e,o){return"symbol"==typeof e&&(e=e.description?"[".concat(e.description,"]"):""),Object.defineProperty(t,"name",{configurable:!0,value:o?"".concat(o," ",e):e})};function o(c,a,r,i,w){return{withTargets:(...t)=>o(c,[...a,...t],r,i,w),withValues:t=>o(c,a,{...r,...t},i,w),withClasses:(...t)=>o(c,a,r,[...i,...t],w),withElementType:()=>o(c,a,r,i,w),withOutlet:t=>o(c,a,r,i,[...w,t.controllerName]),withNamedOutlet:t=>o(c,a,r,i,[...w,t]),build:()=>{var o;return{Base:(o=class extends n{},e(o,"Base"),o.controllerName=c,o.targets=a,o.values=r,o.classes=i,o.outlets=w,o),stimulusCallback:t}}}}const n=function(){const t=new(Stacks.createController({}))({}),e=Object.getPrototypeOf(t),o=Object.getPrototypeOf(e);return Object.getPrototypeOf(o).constructor}(),c=window.Stacks;StackExchange=window.StackExchange=window.StackExchange||{},StackOverflow=window.StackOverflow=window.StackOverflow||{},StackExchange=window.StackExchange=window.StackExchange||{},StackOverflow=window.StackOverflow=window.StackOverflow||{},StackExchange=window.StackExchange=window.StackExchange||{},StackOverflow=window.StackOverflow=window.StackOverflow||{};const{Base:a}=(r="cookie-settings",o(r,[],{},[],[])).withElementType().build();var r;window.OptanonWrapper=()=>{},function(t,e){for(const o of t){if(e&&!o.controllerName.startsWith(e))throw new Error(`The provided Stimulus controllers must be namespaced with "${e}"`);c.application.register(o.controllerName,o)}}([class extends a{toggle(){window.OneTrust.ToggleInfoDisplay()}}],"cookie-")})()})();