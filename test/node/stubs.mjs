// Minimal browser globals so scenerystack and the model code load in Node
const deep = () =>
  new Proxy(function () {}, {
    get: (target, key) =>
      key === Symbol.toPrimitive ? () => ''
      : key === 'length' ? 0
      : key === Symbol.iterator ? function* () {}
      : key === 'then' ? undefined
      : deep(),
    apply: () => deep(),
    construct: () => deep(),
  });
globalThis.addEventListener ??= () => {};
globalThis.removeEventListener ??= () => {};
globalThis.window ??= globalThis;
globalThis.self ??= globalThis;
globalThis.location ??= {
  search: '',
  href: 'http://localhost/',
  hostname: 'localhost',
  origin: 'http://localhost',
  protocol: 'http:',
};
globalThis.document ??= deep();
