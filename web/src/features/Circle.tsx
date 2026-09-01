import { useNavigate, useSearchParams } from 'react-router-dom'
import { PageHeader } from '@/components/ui-bits'
import Galaxy from '@/features/Galaxy'

/**
 * 圈子——独立页（2026-09-01 从用户记忆 tab 拆出）。
 *
 * 语义：记忆模型的「一坐标系」维度——实体（人物/项目/主题/群组/地点）是
 * 透镜，回答「记忆里都有谁/什么事」；用户记忆页（/memory）承载的是
 * 「一架梯子」（L0→L3 蒸馏深度），两个正交视图各自成页。
 * 与侧栏「代码图谱」对称：都是图 + 实体档案型浏览面。
 */
export default function Circle() {
  const navigate = useNavigate()
  const [params] = useSearchParams()
  const entity = params.get('entity')
  return (
    <div className="space-y-6">
      <PageHeader
        title="圈子"
        desc="你的世界里的人与事——按实体浏览记忆，图谱看关系，档案看细节"
      />
      <Galaxy
        key={entity ?? 'none'}
        initialEntity={entity}
        onGoPersona={() => navigate('/memory?tab=persona')}
        onGoAtoms={() => navigate('/memory?tab=atoms')}
      />
    </div>
  )
}
