/**
 * 共享图谱引擎 · 组件（2026-09-21）
 *
 * 一份实现，三处复用（代码图谱 / Wiki 图谱 / 圈子实体星系）。架构与常数照 Obsidian：
 * **sigma(WebGL) 只渲染 + d3-force fork 在 Web Worker 里跑物理**（见 sim.ts / sim.worker.ts）。
 *
 * 对外契约（三种入参都能表达）：
 * - `nodes`：id / label / color（各域自己的语义：角色、社区、实体类型）+ 可选 weight（度数，决定半径）
 *   + 可选 fixed（固定锚点，如圈子的「我」）+ 可选 x/y（**服务端预计算初布局**，有则首帧就位）
 * - `edges`：source / target / 可选 weight、color、width；`directed` 决定箭头与图类型
 * - 交互：hover 高亮邻居、拖拽=钉住(松手解除)、滚轮缩放、拖背景平移、单击/双击回调
 * - 面板：力（向心/斥力/连线拉力/连线长度）+ 显示（节点大小/连线粗细/文字阈值/箭头）+ 过滤
 * - LOD：超阈值自动降级为度数 Top-N 的「简化图」，并给「渲染全图」逃生门
 */
import { useEffect, useMemo, useRef, useState } from 'react'
import Graph from 'graphology'
import Sigma from 'sigma'
import { Maximize, Minimize, ZoomIn, ZoomOut } from 'lucide-react'
import { useThemeTick } from '@/lib/theme'
import { Button } from '@/components/ui/button'
import {
  DEFAULT_DISPLAY,
  DEFAULT_FORCES,
  LOD_THRESHOLDS,
  decideLod,
  degreesOf,
  forcesToWorker,
  labelAlpha,
  labelScale,
  nodeRadius,
  simplifyGraph,
  type DisplayParams,
  type ForceParams,
  type SimOutMsg,
} from './sim'

export interface ForceGraphNode {
  id: string
  label: string
  color: string
  /** 度数（不传则按 edges 现算）——决定半径（Obsidian：被引用越多越大） */
  weight?: number
  /** 固定锚点（不参与物理） */
  fixed?: boolean
  /** 服务端预计算位置：**全部节点都有**时「首帧就位」，只从 reheat alpha 微调 */
  x?: number
  y?: number
}

export interface ForceGraphEdge {
  source: string
  target: string
  weight?: number
  color?: string
  /** 基础线宽（不传按 weight 归一） */
  width?: number
}

export interface ForceGraphProps {
  nodes: ForceGraphNode[]
  edges: ForceGraphEdge[]
  /** 有向图（箭头）？默认无向线 */
  directed?: boolean
  /** 坐标持久化 key（项目 id / 'wiki' / 'circle'） */
  persistKey?: string
  /** 单击节点：交给上层（填查询框 / 跳页面 / 选中实体） */
  onPick?: (id: string) => void
  /** 双击节点：展开它的子图（codegraph 的「Local graph」语义；无则忽略） */
  onExpand?: (id: string) => void
  /** 顶部工具条（各域自加：刷新图 / 全屏 / 着色切换…） */
  toolbar?: React.ReactNode
  /** 底部图例（各域自加语义） */
  legend?: React.ReactNode
  /** 高亮节点集合（wiki 洞察联动等） */
  highlightIds?: string[]
  initialForces?: Partial<ForceParams>
  initialDisplay?: Partial<DisplayParams>
  /** 强制全量（用户点了「渲染全图」后由上层记住） */
  forceFull?: boolean
  onForceFull?: () => void
  /** 画布 testid（各域自定，便于 e2e 定位；默认 force-graph-canvas） */
  testId?: string
  /** 全屏目标：不传 = 全屏引擎自己这块；传了 = 整块进全屏（如 Wiki 把「着色切换」也带进去） */
  fullscreenTargetRef?: React.RefObject<HTMLElement | null>
  className?: string
}

const ZOOM_BTN =
  'flex size-7 items-center justify-center rounded-md border border-border bg-card text-muted-foreground shadow-sm transition-colors hover:border-foreground/30 hover:text-foreground'

export default function ForceGraph({
  nodes,
  edges,
  directed = false,
  persistKey,
  onPick,
  onExpand,
  toolbar,
  legend,
  highlightIds,
  initialForces,
  initialDisplay,
  forceFull,
  onForceFull,
  testId,
  fullscreenTargetRef,
  className,
}: ForceGraphProps) {
  const canvasRef = useRef<HTMLDivElement>(null)
  const rootRef = useRef<HTMLDivElement>(null)
  const sigmaRef = useRef<Sigma | null>(null)
  const workerRef = useRef<Worker | null>(null)
  const themeTick = useThemeTick()

  const forcesRef = useRef<ForceParams>({ ...DEFAULT_FORCES, ...initialForces })
  const displayRef = useRef<DisplayParams>({ ...DEFAULT_DISPLAY, arrows: directed, ...initialDisplay })
  const filterRef = useRef('')
  const [panelOpen, setPanelOpen] = useState(true)
  const [showDisplay, setShowDisplay] = useState(false)
  const [physics, setPhysics] = useState<'running' | 'idle'>('running')
  const [showFullAnyway, setShowFullAnyway] = useState(forceFull ?? false)
  /** 用户显式点「只看主干」——与「渲染全图」互斥（后点者胜） */
  const [forceSimplify, setForceSimplify] = useState(false)
  /** 当前是否全屏（全屏元素 = fullscreenTargetRef ?? 引擎根节点） */
  const [isFs, setIsFs] = useState(false)

  // ── LOD 决策（超阈值自动降级；用户可显式「只看主干」/「渲染全图」） ──
  const lod = useMemo(
    () =>
      decideLod(
        nodes.length,
        edges.length,
        showFullAnyway ? 'full' : forceSimplify ? 'simplified' : undefined,
      ),
    [nodes.length, edges.length, showFullAnyway, forceSimplify],
  )
  const view = useMemo(() => {
    const simplified = simplifyGraph(
      nodes.map((n) => ({ id: n.id, weight: n.weight, fixed: n.fixed, x: n.x, y: n.y })),
      edges.map((e) => ({ source: e.source, target: e.target, weight: e.weight })),
      lod,
    )
    const keep = new Set(simplified.nodes.map((n) => n.id))
    return {
      nodes: nodes.filter((n) => keep.has(n.id)),
      edges: edges.filter((e) => keep.has(e.source) && keep.has(e.target)),
    }
  }, [nodes, edges, lod])

  /** 度数（半径用；被引用越多越大） */
  const degree = useMemo(
    () => degreesOf(view.edges.map((e) => ({ source: e.source, target: e.target }))),
    [view.edges],
  )
  const posKey = persistKey ? `forcegraph:${persistKey}` : null

  // ── 全屏（三处共用）：元素级 requestFullscreen；进出后派发 resize——sigma 依容器尺寸重排 ──
  useEffect(() => {
    const onFs = () => {
      setIsFs(document.fullscreenElement === (fullscreenTargetRef?.current ?? rootRef.current))
      // 进全屏容器尺寸变了：派发 resize 让 sigma 重新量一次（沿用 CodeGraph 既有口径）
      requestAnimationFrame(() => window.dispatchEvent(new Event('resize')))
    }
    document.addEventListener('fullscreenchange', onFs)
    return () => document.removeEventListener('fullscreenchange', onFs)
  }, [fullscreenTargetRef])

  const toggleFs = () => {
    const el = fullscreenTargetRef?.current ?? rootRef.current
    if (!el) return
    if (document.fullscreenElement) void document.exitFullscreen()
    else void el.requestFullscreen()
  }

  // ── 主 effect：建图 + sigma + worker（graph/主题变化才重建） ──
  useEffect(() => {
    const el = canvasRef.current
    if (!el || view.nodes.length === 0) return

    const css = getComputedStyle(document.documentElement)
    const tc = (name: string, fb: string) => css.getPropertyValue(name).trim() || fb
    const labelColor = tc('--foreground', '#0a0a0a')
    const edgeBase = tc('--border', '#e5e5e5')
    const edgeHi = tc('--muted-foreground', '#636365')
    const highlight = tc('--foreground', '#0a0a0a')

    // 服务端初布局：全部节点都带 x/y 才算「首帧就位」
    const laidOut = view.nodes.length > 0 && view.nodes.every((n) => n.x !== undefined && n.y !== undefined)
    const cached: Record<string, { x: number; y: number }> = (() => {
      if (!posKey) return {}
      try {
        return JSON.parse(localStorage.getItem(posKey) ?? '{}')
      } catch {
        return {}
      }
    })()

    const g = new Graph({ multi: false, type: directed ? 'directed' : 'undirected' })
    const hi = new Set(highlightIds ?? [])
    const n = view.nodes.length
    view.nodes.forEach((node, i) => {
      const deg = node.weight ?? degree.get(node.id) ?? 0
      const cachedPos = cached[node.id]
      const angle = (2 * Math.PI * i) / n
      g.addNode(node.id, {
        label: node.label,
        baseColor: node.color,
        color: hi.has(node.id) ? highlight : node.color,
        weight: deg,
        baseSize: nodeRadius(deg, 1),
        size: nodeRadius(deg, displayRef.current.nodeSize),
        // 优先级：服务端初布局 > 本地缓存 > 环形随机（big bang 开场）
        x: node.x ?? cachedPos?.x ?? Math.cos(angle) * (1 + Math.random()),
        y: node.y ?? cachedPos?.y ?? Math.sin(angle) * (1 + Math.random()),
        fixed: node.fixed ?? false,
      })
    })
    const maxW = Math.max(1, ...view.edges.map((e) => e.weight ?? 1))
    for (const e of view.edges) {
      if (!g.hasNode(e.source) || !g.hasNode(e.target)) continue
      if (directed ? g.hasDirectedEdge(e.source, e.target) : g.hasEdge(e.source, e.target)) continue
      const w = (e.weight ?? 1) / maxW
      const base = e.width ?? 0.4 + Math.min(w, 1) * 2
      const attrs = {
        baseSize: base,
        size: base * displayRef.current.lineSize,
        color: e.color ?? edgeBase,
        weight: e.weight ?? 1,
      }
      if (directed) g.addDirectedEdge(e.source, e.target, attrs)
      else g.addEdge(e.source, e.target, attrs)
    }

    // hover/拖拽状态（reducer 声明式消费，不遍历改属性）
    const state = { hover: null as string | null, drag: null as string | null }
    const neighbors = new Map<string, Set<string>>()
    g.forEachNode((id) => neighbors.set(id, new Set(g.neighbors(id))))
    const active = () => state.hover ?? state.drag
    const faded = (c: string) => `${c.slice(0, 7)}33`

    // 相机缩放比：**reducer 里绝不能读 sigma**——`new Sigma(...)` 构造期就会调用本 reducer，
    // 那时 `sigma` 还在 TDZ（线上实测：「Cannot access 'N' before initialization」，
    // 页面整块白）。改成闭包变量，由 camera 'updated' 事件更新。
    let camRatio = 1
    // sigma 的 ratio 越大越「缩小」；labelAlpha 要放大倍数 → 取倒数
    const labelVisible = () => labelAlpha(1 / camRatio, displayRef.current.textFade) >= 0.15

    const sigma = new Sigma(g, el, {
      allowInvalidContainer: true,
      renderEdgeLabels: false,
      defaultEdgeType: directed && displayRef.current.arrows ? 'arrow' : 'line',
      minCameraRatio: 1 / 128,
      maxCameraRatio: 8,
      labelRenderedSizeThreshold: 2,
      labelColor: { color: labelColor },
      labelWeight: '500',
      nodeReducer: (id, data) => {
        const a = active()
        const f = filterRef.current.trim().toLowerCase()
        const out: Record<string, unknown> = {
          size: ((data.baseSize as number) ?? 4) * displayRef.current.nodeSize,
          label: labelVisible() ? data.label : null,
          labelSize: 12 * labelScale(camRatio),
        }
        if (f && !String(data.label ?? '').toLowerCase().includes(f)) {
          return { ...data, ...out, color: faded(((data.baseColor as string) ?? '#888') + ''), zIndex: 0 }
        }
        if (!a || id === a) return { ...data, ...out, zIndex: 2 }
        if (neighbors.get(a)?.has(id)) return { ...data, ...out, zIndex: 1 }
        return { ...data, ...out, color: faded((data.baseColor as string) ?? '#888'), label: null, zIndex: 0 }
      },
      edgeReducer: (edge, data) => {
        const a = active()
        const size = ((data.baseSize as number) ?? 1) * displayRef.current.lineSize
        if (!a) return { ...data, size, color: `${(data.color as string) ?? edgeBase}` }
        const [x, y] = g.extremities(edge)
        if (x === a || y === a) return { ...data, size: Math.max(size, 1.6), color: edgeHi }
        return { ...data, hidden: true }
      },
    })
    sigmaRef.current = sigma

    // ── 物理：Web Worker（d3-force fork；常数见 sim.ts） ──
    const worker = new Worker(new URL('./sim.worker.ts', import.meta.url), { type: 'module' })
    workerRef.current = worker
    const order = view.nodes.map((v) => v.id)
    const posOf = (id: string) => {
      const attrs = g.getNodeAttributes(id) as { x: number; y: number }
      return { x: attrs.x, y: attrs.y }
    }
    let pendingPositions: Float32Array | null = null
    let rafId = 0
    /** worker 宣告停表（alpha < alphaMin）后置真——坐标持久化只在真静止时存一次 */
    let idleSeen = false
    const flush = () => {
      rafId = 0
      const pos = pendingPositions
      pendingPositions = null
      if (!pos) return
      for (let i = 0; i < order.length; i++) {
        const id = order[i]
        if (!g.hasNode(id)) continue
        const x = pos[i * 2]
        const y = pos[i * 2 + 1]
        if (Number.isFinite(x) && Number.isFinite(y)) {
          g.setNodeAttribute(id, 'x', x)
          g.setNodeAttribute(id, 'y', y)
        }
      }
      sigma.refresh({ skipIndexation: true })
    }
    worker.onmessage = (ev: MessageEvent<SimOutMsg>) => {
      const msg = ev.data
      if (msg.type === 'tick') {
        pendingPositions = msg.positions
        setPhysics('running')
        if (!rafId) rafId = requestAnimationFrame(flush)
      } else if (msg.type === 'end') {
        // 不动就真静止：worker 侧 d3 simulation 已停表（alpha < alphaMin）、不再产生 tick
        setPhysics('idle')
        idleSeen = true
      }
    }
    worker.postMessage({
      type: 'init',
      nodes: view.nodes.map((v) => ({
        id: v.id,
        weight: v.weight ?? degree.get(v.id) ?? 0,
        fixed: v.fixed ?? false,
        x: posOf(v.id).x,
        y: posOf(v.id).y,
      })),
      edges: view.edges.map((e) => ({
        source: e.source,
        target: e.target,
        weight: e.weight ?? 1,
      })),
      forces: forcesToWorker(forcesRef.current),
      // 有初布局 → 只从 reheat 微调；否则从头收敛
      alpha: laidOut ? 0.3 : 1,
    })

    // 打开时把视野拉到整图（约 300ms 后，等布局铺开）
    const fitTimer = setTimeout(() => sigma.getCamera().animatedReset({ duration: 400 }), laidOut ? 60 : 700)

    // ── 交互：拖拽=钉住（松手解除）、hover、单击/双击、相机变动时刷新标签 ──
    const captor = sigma.getMouseCaptor()
    let dragging: string | null = null
    sigma.on('downNode', ({ node }) => {
      dragging = node
      state.drag = node
      captor.enabled = false // 拖节点时不跟着平移相机
      const p = posOf(node)
      worker.postMessage({ type: 'pin', id: node, x: p.x, y: p.y })
    })
    const onMove = (ev: MouseEvent) => {
      if (!dragging) return
      const rect = el.getBoundingClientRect()
      const p = sigma.viewportToGraph({ x: ev.clientX - rect.left, y: ev.clientY - rect.top })
      g.setNodeAttribute(dragging, 'x', p.x)
      g.setNodeAttribute(dragging, 'y', p.y)
      worker.postMessage({ type: 'pin', id: dragging, x: p.x, y: p.y })
      sigma.refresh({ skipIndexation: true })
    }
    const onUp = () => {
      if (dragging) {
        worker.postMessage({ type: 'unpin', id: dragging })
        sigma.refresh({ skipIndexation: true })
      }
      dragging = null
      state.drag = null
      captor.enabled = true
    }
    el.addEventListener('mousemove', onMove)
    el.addEventListener('mouseup', onUp)

    sigma.on('enterNode', ({ node }) => {
      state.hover = node
      sigma.refresh({ skipIndexation: true })
    })
    sigma.on('leaveNode', () => {
      state.hover = null
      sigma.refresh({ skipIndexation: true })
    })
    sigma.on('clickNode', ({ node }) => onPick?.(node))
    sigma.on('doubleClickNode', ({ node, event }) => {
      event.preventSigmaDefault?.()
      onExpand?.(node)
    })

    // 相机变动 → 标签按 labelAlpha 淡入淡出（节流）
    let camTimer = 0
    const onCam = () => {
      if (camTimer) return
      camTimer = window.setTimeout(() => {
        camTimer = 0
        camRatio = sigma.getCamera().ratio
        sigma.refresh({ skipIndexation: true })
      }, 120)
    }
    sigma.getCamera().on('updated', onCam)

    // ── 坐标持久化：物理停表后存一次 + 卸载时存 ──
    let savedIdle = false
    const save = () => {
      if (!posKey) return
      try {
        const out: Record<string, { x: number; y: number }> = {}
        g.forEachNode((id, attrs) => {
          out[id] = { x: attrs.x as number, y: attrs.y as number }
        })
        localStorage.setItem(posKey, JSON.stringify(out))
      } catch {
        /* 隐私模式等：存不下就算了 */
      }
    }
    const saveTick = setInterval(() => {
      if (idleSeen && !savedIdle) {
        save()
        savedIdle = true
      }
    }, 3000)

    return () => {
      clearTimeout(fitTimer)
      clearInterval(saveTick)
      if (camTimer) clearTimeout(camTimer)
      save()
      if (rafId) cancelAnimationFrame(rafId)
      el.removeEventListener('mousemove', onMove)
      el.removeEventListener('mouseup', onUp)
      worker.postMessage({ type: 'stop' })
      worker.terminate()
      workerRef.current = null
      sigma.kill()
      sigmaRef.current = null
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [view.nodes, view.edges, directed, themeTick, posKey])

  /** 参数改动：写 ref → 下发 worker（力）或只刷新（显示）。 */
  const pushForces = () => {
    workerRef.current?.postMessage({ type: 'params', forces: forcesToWorker(forcesRef.current) })
    sigmaRef.current?.refresh({ skipIndexation: true })
  }
  const pushDisplay = () => sigmaRef.current?.refresh({ skipIndexation: true })

  const forceSlider = (key: keyof ForceParams, label: string, min: number, max: number, step: number) => (
    <label key={key} className="flex items-center gap-1.5 text-[10px] text-muted-foreground">
      <span className="w-14 shrink-0">{label}</span>
      <input
        type="range"
        min={min}
        max={max}
        step={step}
        defaultValue={DEFAULT_FORCES[key]}
        onChange={(e) => {
          forcesRef.current[key] = Number(e.target.value)
          pushForces()
        }}
        className="h-1 w-16 cursor-pointer accent-foreground"
      />
    </label>
  )
  const displaySlider = (key: keyof DisplayParams, label: string, min: number, max: number, step: number) => (
    <label key={key} className="flex items-center gap-1.5 text-[10px] text-muted-foreground">
      <span className="w-14 shrink-0">{label}</span>
      <input
        type="range"
        min={min}
        max={max}
        step={step}
        defaultValue={DEFAULT_DISPLAY[key] as number}
        onChange={(e) => {
          ;(displayRef.current[key] as number) = Number(e.target.value)
          pushDisplay()
        }}
        className="h-1 w-16 cursor-pointer accent-foreground"
      />
    </label>
  )

  if (nodes.length === 0) return null

  return (
    // 根必须是 **flex 列**：否则里面 `flex-1` 的画布包裹层没有可分配的父容器，只能退回画布最小高度
    // （实测症状：容器 538px、画布 280px，下方留一大块死白——用户 2026-09-21 报）
    <div
      ref={rootRef}
      className={`flex min-h-0 flex-col ${isFs ? 'h-full w-full bg-background p-3' : ''} ${
        className ?? 'relative w-full flex-1'
      }`}
    >
      {toolbar && <div className="mb-2 flex flex-wrap items-center gap-2">{toolbar}</div>}
      <div className="relative min-h-0 flex-1">
        <div
          ref={canvasRef}
          className="wiki-graph-canvas h-full min-h-[280px] w-full overflow-hidden rounded-md border border-border bg-card"
          role="img"
          tabIndex={0}
          data-testid={testId ?? 'force-graph-canvas'}
          aria-label="力导图谱：滚轮缩放、拖背景平移、拖节点跟手（松手回弹）、悬停高亮邻居、单击选中、双击展开子图。右上「参数」可调力与显示。"
        />
        {/* 右上：过滤 + 参数（力 / 显示，Obsidian 齿轮里的三组） */}
        <div className="absolute top-2 right-2 flex flex-col items-end gap-1.5">
          <div className="flex items-center gap-1.5">
            <input
              className="h-6 w-32 rounded-md border border-border bg-card/90 px-2 text-[11px] shadow-sm backdrop-blur"
              placeholder="过滤节点…"
              aria-label="过滤节点"
              onChange={(e) => {
                filterRef.current = e.target.value
                pushDisplay()
              }}
            />
            <button
              type="button"
              onClick={() => setPanelOpen((v) => !v)}
              aria-expanded={panelOpen}
              className="rounded-md border border-border bg-card/90 px-2 py-1 text-[11px] text-muted-foreground shadow-sm backdrop-blur hover:text-foreground"
              title="力 / 显示 参数（对照 Obsidian）"
            >
              参数 {panelOpen ? '▴' : '▾'}
            </button>
            <button
              type="button"
              onClick={toggleFs}
              data-testid="force-graph-fullscreen"
              aria-label={isFs ? '退出全屏' : '全屏'}
              className="flex items-center gap-1 rounded-md border border-border bg-card/90 px-2 py-1 text-[11px] text-muted-foreground shadow-sm backdrop-blur hover:text-foreground"
              title={isFs ? '退出全屏（Esc）' : '全屏看整张图（Esc 退出）'}
            >
              {isFs ? <Minimize className="size-3" /> : <Maximize className="size-3" />}
              {isFs ? '退出全屏' : '全屏'}
            </button>
          </div>
          {panelOpen && (
            <div className="flex flex-col gap-1.5 rounded-lg border border-border bg-card/90 px-3 py-2 shadow-sm backdrop-blur">
              <p className="text-[10px] font-medium text-muted-foreground">
                力 · Forces
                <span className="ml-1.5 font-normal">
                  {physics === 'idle' ? '（已静止）' : `（运行中 alpha）`}
                </span>
              </p>
              {forceSlider('center', '向心', 0, 1, 0.01)}
              {forceSlider('repel', '斥力', 0, 20, 0.5)}
              {forceSlider('linkStrength', '连线拉力', 0, 1, 0.01)}
              {forceSlider('linkDistance', '连线长度', 30, 500, 5)}
              <button
                type="button"
                className="mt-0.5 text-left text-[10px] text-muted-foreground hover:text-foreground"
                onClick={() => setShowDisplay((v) => !v)}
                aria-expanded={showDisplay}
              >
                显示 · Display {showDisplay ? '▴' : '▾'}
              </button>
              {showDisplay && (
                <div className="flex flex-col gap-1.5">
                  {displaySlider('nodeSize', '节点大小', 0.1, 5, 0.1)}
                  {displaySlider('lineSize', '连线粗细', 0.1, 5, 0.1)}
                  {displaySlider('textFade', '文字阈值', -3, 3, 0.1)}
                  <label className="flex items-center gap-1.5 text-[10px] text-muted-foreground">
                    <span className="w-14 shrink-0">箭头</span>
                    <input
                      type="checkbox"
                      defaultChecked={directed && DEFAULT_DISPLAY.arrows}
                      onChange={(e) => {
                        const on = e.target.checked
                        displayRef.current.arrows = on
                        sigmaRef.current?.setSetting('defaultEdgeType', on ? 'arrow' : 'line')
                        pushDisplay()
                      }}
                      className="size-3 cursor-pointer accent-foreground"
                    />
                  </label>
                </div>
              )}
            </div>
          )}
        </div>
        {/* 左下：布局动作 */}
        <div className="absolute bottom-2 left-2 flex gap-2">
          {lod.mode === 'full' && nodes.length > LOD_THRESHOLDS.suggestNodes && (
            <button
              type="button"
              className="rounded-md border border-border bg-card/90 px-2 py-1 text-[11px] text-muted-foreground shadow-sm backdrop-blur transition-colors hover:text-foreground"
              title={`图较大（${nodes.length} 个节点）——只看度数最高的主干，随时可点「渲染全图」回来`}
              onClick={() => {
                setShowFullAnyway(false)
                setForceSimplify(true)
              }}
            >
              只看主干
            </button>
          )}
          <button
            type="button"
            className="rounded-md border border-border bg-card/90 px-2 py-1 text-[11px] text-muted-foreground shadow-sm backdrop-blur transition-colors hover:text-foreground"
            title="忘掉记住的坐标，重新炸开一次布局"
            onClick={() => {
              if (posKey) localStorage.removeItem(posKey)
              workerRef.current?.postMessage({ type: 'reheat', alpha: 1 })
              sigmaRef.current?.getCamera().animatedReset({ duration: 300 })
            }}
          >
            重排布局
          </button>
          <button
            type="button"
            className="rounded-md border border-border bg-card/90 px-2 py-1 text-[11px] text-muted-foreground shadow-sm backdrop-blur transition-colors hover:text-foreground"
            title="把视野拉回整张图"
            onClick={() => sigmaRef.current?.getCamera().animatedReset({ duration: 300 })}
          >
            重置视图
          </button>
        </div>
        {/* 右下：缩放 */}
        <div className="absolute right-2 bottom-2 flex flex-col gap-1">
          <button
            type="button"
            title="放大"
            aria-label="放大"
            className={ZOOM_BTN}
            onClick={() => sigmaRef.current?.getCamera().animatedZoom({ duration: 200, factor: 1.4 })}
          >
            <ZoomIn className="size-3.5" />
          </button>
          <button
            type="button"
            title="缩小"
            aria-label="缩小"
            className={ZOOM_BTN}
            onClick={() => sigmaRef.current?.getCamera().animatedZoom({ duration: 200, factor: 1 / 1.4 })}
          >
            <ZoomOut className="size-3.5" />
          </button>
          <button
            type="button"
            title="适应屏幕"
            aria-label="适应屏幕"
            className={ZOOM_BTN}
            onClick={() => sigmaRef.current?.getCamera().animatedReset({ duration: 300 })}
          >
            <Maximize className="size-3.5" />
          </button>
        </div>
        {/* LOD 降级提示 + 逃生门（大图不再「要么手点、要么卡住」） */}
        {lod.mode === 'simplified' && (
          <div className="absolute bottom-2 left-1/2 flex -translate-x-1/2 items-center gap-3 rounded-md border border-border bg-card/95 px-3 py-1.5 text-[11px] shadow-sm backdrop-blur">
            <span className="text-muted-foreground">{lod.reason}</span>
            <Button
              size="sm"
              variant="outline"
              onClick={() => {
                setForceSimplify(false)
                setShowFullAnyway(true)
                onForceFull?.()
              }}
            >
              渲染全图
            </Button>
          </div>
        )}
      </div>
      {legend && <div className="mt-2 flex flex-wrap items-center gap-3 text-[11px] text-muted-foreground">{legend}</div>}
    </div>
  )
}
