"use strict";(self.webpackChunkwebsite=self.webpackChunkwebsite||[]).push([[4525],{3905:(e,t,r)=>{r.r(t),r.d(t,{MDXContext:()=>c,MDXProvider:()=>d,mdx:()=>b,useMDXComponents:()=>u,withMDXComponents:()=>s});var n=r(67294);function a(e,t,r){return t in e?Object.defineProperty(e,t,{value:r,enumerable:!0,configurable:!0,writable:!0}):e[t]=r,e}function o(){return o=Object.assign||function(e){for(var t=1;t<arguments.length;t++){var r=arguments[t];for(var n in r)Object.prototype.hasOwnProperty.call(r,n)&&(e[n]=r[n])}return e},o.apply(this,arguments)}function l(e,t){var r=Object.keys(e);if(Object.getOwnPropertySymbols){var n=Object.getOwnPropertySymbols(e);t&&(n=n.filter((function(t){return Object.getOwnPropertyDescriptor(e,t).enumerable}))),r.push.apply(r,n)}return r}function i(e){for(var t=1;t<arguments.length;t++){var r=null!=arguments[t]?arguments[t]:{};t%2?l(Object(r),!0).forEach((function(t){a(e,t,r[t])})):Object.getOwnPropertyDescriptors?Object.defineProperties(e,Object.getOwnPropertyDescriptors(r)):l(Object(r)).forEach((function(t){Object.defineProperty(e,t,Object.getOwnPropertyDescriptor(r,t))}))}return e}function p(e,t){if(null==e)return{};var r,n,a=function(e,t){if(null==e)return{};var r,n,a={},o=Object.keys(e);for(n=0;n<o.length;n++)r=o[n],t.indexOf(r)>=0||(a[r]=e[r]);return a}(e,t);if(Object.getOwnPropertySymbols){var o=Object.getOwnPropertySymbols(e);for(n=0;n<o.length;n++)r=o[n],t.indexOf(r)>=0||Object.prototype.propertyIsEnumerable.call(e,r)&&(a[r]=e[r])}return a}var c=n.createContext({}),s=function(e){return function(t){var r=u(t.components);return n.createElement(e,o({},t,{components:r}))}},u=function(e){var t=n.useContext(c),r=t;return e&&(r="function"==typeof e?e(t):i(i({},t),e)),r},d=function(e){var t=u(e.components);return n.createElement(c.Provider,{value:t},e.children)},y="mdxType",m={inlineCode:"code",wrapper:function(e){var t=e.children;return n.createElement(n.Fragment,{},t)}},f=n.forwardRef((function(e,t){var r=e.components,a=e.mdxType,o=e.originalType,l=e.parentName,c=p(e,["components","mdxType","originalType","parentName"]),s=u(r),d=a,y=s["".concat(l,".").concat(d)]||s[d]||m[d]||o;return r?n.createElement(y,i(i({ref:t},c),{},{components:r})):n.createElement(y,i({ref:t},c))}));function b(e,t){var r=arguments,a=t&&t.mdxType;if("string"==typeof e||a){var o=r.length,l=new Array(o);l[0]=f;var i={};for(var p in t)hasOwnProperty.call(t,p)&&(i[p]=t[p]);i.originalType=e,i[y]="string"==typeof e?e:a,l[1]=i;for(var c=2;c<o;c++)l[c]=r[c];return n.createElement.apply(null,l)}return n.createElement.apply(null,r)}f.displayName="MDXCreateElement"},75562:(e,t,r)=>{r.r(t),r.d(t,{assets:()=>p,contentTitle:()=>l,default:()=>u,frontMatter:()=>o,metadata:()=>i,toc:()=>c});var n=r(87462),a=(r(67294),r(3905));const o={id:"lazy_attrs"},l="lazy_attrs type",i={unversionedId:"api/bxl/lazy_attrs",id:"api/bxl/lazy_attrs",title:"lazy_attrs type",description:"The context for getting attrs lazily on a StarlarkConfiguredTargetNode.",source:"@site/../docs/api/bxl/lazy_attrs.generated.md",sourceDirName:"api/bxl",slug:"/api/bxl/lazy_attrs",permalink:"/docs/api/bxl/lazy_attrs",draft:!1,tags:[],version:"current",frontMatter:{id:"lazy_attrs"},sidebar:"manualSidebar",previous:{title:"instant type",permalink:"/docs/api/bxl/instant"},next:{title:"lazy_resolved_attrs type",permalink:"/docs/api/bxl/lazy_resolved_attrs"}},p={},c=[{value:"lazy_attrs.get",id:"lazy_attrsget",level:2}],s={toc:c};function u(e){let{components:t,...r}=e;return(0,a.mdx)("wrapper",(0,n.Z)({},s,r,{components:t,mdxType:"MDXLayout"}),(0,a.mdx)("h1",{id:"lazy_attrs-type"},(0,a.mdx)("inlineCode",{parentName:"h1"},"lazy_attrs")," type"),(0,a.mdx)("p",null,"The context for getting attrs lazily on a ",(0,a.mdx)("inlineCode",{parentName:"p"},"StarlarkConfiguredTargetNode"),"."),(0,a.mdx)("h2",{id:"lazy_attrsget"},"lazy","_","attrs.get"),(0,a.mdx)("pre",null,(0,a.mdx)("code",{parentName:"pre",className:"language-python"},"def lazy_attrs.get(attr: str) -> None | configured_attr\n")),(0,a.mdx)("p",null,"Gets a single attribute. Returns an optional ",(0,a.mdx)("inlineCode",{parentName:"p"},"[StarlarkConfiguredAttr]"),"."),(0,a.mdx)("p",null,'def _impl_attrs_lazy(ctx):\nnode = ctx.cquery().owner("cell//path/to/TARGETS")',"[0]",'\nattrs = node.attrs_lazy() # cache once\nctx.output.print(attrs.get("some_attributes").value())\nctx.output.print(attrs.get("some_attribute").label)'),(0,a.mdx)("pre",null,(0,a.mdx)("code",{parentName:"pre"},"")))}u.isMDXComponent=!0}}]);