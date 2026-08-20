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
