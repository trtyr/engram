/** 主题切换按钮：壳与登录页共用。iconClass 供收起窄轨放大图标。 */
import { Moon, Sun } from 'lucide-react'
import { useTheme } from '@/lib/theme'
import { Button } from '@/components/ui/button'

export function ThemeToggle({ iconClass = 'size-4' }: { iconClass?: string }) {
  const { theme, toggle } = useTheme()
  return (
    <Button
      variant="ghost"
      size="icon-sm"
      aria-label={theme === 'dark' ? '切换到亮色' : '切换到暗色'}
      onClick={toggle}
    >
      {theme === 'dark' ? <Sun className={iconClass} /> : <Moon className={iconClass} />}
    </Button>
  )
}
