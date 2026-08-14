import { useEffect, useState } from 'react'
import { Gauge } from 'lucide-react'
import { toast } from 'sonner'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Switch } from '@/components/ui/switch'
import {
  Dialog, DialogContent, DialogHeader, DialogTitle, DialogDescription, DialogFooter,
} from '@/components/ui/dialog'
import { useAccountRpmConfig, useSetAccountRpmConfig } from '@/hooks/use-credentials'
import { extractErrorMessage } from '@/lib/utils'

interface AccountRpmDialogProps {
  open: boolean
  onOpenChange: (open: boolean) => void
}

/** 常用上限预设，省去手输 */
const PRESETS = [10, 30, 60, 120, 300]

const MAX_LIMIT = 100_000

/**
 * 单账号 RPM 主动限流。
 *
 * 每个凭据独立维护 60 秒滑动窗口，达到上限后临时退出候选、请求自动转到下一个
 * 可用账号；所有匹配账号都耗尽时返回标准 429 + Retry-After。
 */
export function AccountRpmDialog({ open, onOpenChange }: AccountRpmDialogProps) {
  const { data, isLoading } = useAccountRpmConfig()
  const { mutate: save, isPending: saving } = useSetAccountRpmConfig()

  const [enabled, setEnabled] = useState(false)
  const [limit, setLimit] = useState('60')

  useEffect(() => {
    if (data) {
      setEnabled(data.enabled)
      setLimit(String(data.limit))
    }
  }, [data])

  const handleSave = () => {
    const value = Number(limit)
    if (!Number.isInteger(value) || value < 0 || value > MAX_LIMIT) {
      toast.error(`每分钟上限需为 0-${MAX_LIMIT} 之间的整数`)
      return
    }
    save(
      { enabled, limit: value },
      {
        onSuccess: (saved) => {
          toast.success(
            saved.enabled
              ? `RPM 限流已启用（每账号每分钟 ${saved.limit === 0 ? '不限' : saved.limit} 次）`
              : 'RPM 限流已关闭',
          )
          onOpenChange(false)
        },
        onError: (err) => toast.error(`保存失败: ${extractErrorMessage(err)}`),
      },
    )
  }

  const disabled = isLoading || saving

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-lg">
        <DialogHeader>
          <DialogTitle className="flex items-center gap-2">
            <Gauge className="h-4 w-4" />
            单账号请求限流
          </DialogTitle>
          <DialogDescription>
            给每个账号加一个 60 秒滑动窗口的请求上限。达到上限的账号临时退出候选，
            请求自动转到下一个可用账号；所有匹配账号都用满时返回 429 并附上
            Retry-After，而不是报「所有凭据均已禁用」。
          </DialogDescription>
        </DialogHeader>

        <div className="space-y-4">
          <div className="flex items-center justify-between gap-3 rounded-md bg-secondary/40 px-3 py-2.5">
            <div className="text-xs">
              <div className="font-medium text-foreground">
                {enabled ? '已启用' : '已关闭'}
              </div>
              <div className="leading-snug text-muted-foreground">
                {enabled
                  ? '按下面的上限主动限流，避免单账号短时间内请求过密'
                  : '不做主动限流，调度行为与未启用本功能时一致'}
              </div>
            </div>
            <Switch checked={enabled} disabled={disabled} onCheckedChange={setEnabled} />
          </div>

          <div className="space-y-2">
            <div className="flex items-center gap-2">
              <label className="w-24 shrink-0 text-xs font-medium text-foreground">
                每分钟上限
              </label>
              <Input
                type="number"
                min={0}
                max={MAX_LIMIT}
                value={limit}
                disabled={disabled || !enabled}
                onChange={(e) => setLimit(e.target.value)}
                className="h-7 text-xs"
              />
              <span className="shrink-0 text-xs text-muted-foreground">次 / 账号</span>
            </div>
            <div className="flex flex-wrap gap-1.5 pl-24">
              {PRESETS.map((preset) => (
                <Button
                  key={preset}
                  variant={Number(limit) === preset ? 'default' : 'outline'}
                  size="sm"
                  className="h-6 px-2 text-xs"
                  disabled={disabled || !enabled}
                  onClick={() => setLimit(String(preset))}
                >
                  {preset}
                </Button>
              ))}
            </div>
            <p className="pl-24 text-xs leading-snug text-muted-foreground">
              只统计真实业务请求。设为 0 等同不限。
            </p>
          </div>
        </div>

        <DialogFooter>
          <Button variant="outline" size="sm" disabled={saving} onClick={() => onOpenChange(false)}>
            取消
          </Button>
          <Button size="sm" disabled={disabled} onClick={handleSave}>
            {saving ? '保存中…' : '保存'}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}
