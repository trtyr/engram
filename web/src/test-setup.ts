import '@testing-library/jest-dom/vitest'

// jsdom 无 WebGL/canvas：sigma 导入期探测 renderer 会崩，测试前 stub 掉
class WebGL2ContextStub {
  getParameter() { return 4096 }
  getExtension() { return null }
  getContextAttributes() { return {} }
  createShader() { return {} }
  shaderSource() {}
  compileShader() {}
  getShaderParameter() { return true }
  createProgram() { return {} }
  attachShader() {}
  linkProgram() {}
  getProgramParameter() { return true }
  useProgram() {}
  createBuffer() { return {} }
  bindBuffer() {}
  bufferData() {}
  enableVertexAttribArray() {}
  vertexAttribPointer() {}
  drawArrays() {}
  viewport() {}
  clearColor() {}
  clear() {}
}
if (typeof globalThis.WebGL2RenderingContext === 'undefined') {
  ;(globalThis as unknown as Record<string, unknown>).WebGL2RenderingContext = WebGL2ContextStub
}
if (typeof globalThis.WebGLRenderingContext === 'undefined') {
  ;(globalThis as unknown as Record<string, unknown>).WebGLRenderingContext = WebGL2ContextStub
}
if (typeof globalThis.HTMLCanvasElement !== 'undefined' && !HTMLCanvasElement.prototype.getContext) {
  // 仅 stub webgl 分支；2d 等其他重载声明为可空实现
  (HTMLCanvasElement.prototype as unknown as { getContext: (id: string) => unknown }).getContext = (id: string) =>
    id.startsWith('webgl') ? (new WebGL2ContextStub() as unknown as WebGL2RenderingContext) : null
}

// jsdom 无 ResizeObserver：ClampText（web/src/components/ClampText.tsx）用它做溢出检测，
// 不 stub 会在 useLayoutEffect 里抛 ReferenceError，导致依赖它的用例整片崩。
// 这里给空实现：初次 check() 仍会执行，只是不再跟随尺寸变化重测（jsdom 无真实布局，
// scrollHeight/clientHeight 恒为 0 → 判为未溢出，符合测试预期）。
class ResizeObserverStub {
  observe() {}
  unobserve() {}
  disconnect() {}
}
if (typeof globalThis.ResizeObserver === 'undefined') {
  ;(globalThis as unknown as Record<string, unknown>).ResizeObserver = ResizeObserverStub
}
