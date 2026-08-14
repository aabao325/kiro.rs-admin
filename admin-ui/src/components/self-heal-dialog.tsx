import { useEffect, useState } from 'react'
import { HeartPulse } from 'lucide-react'
import { toast } from 'sonner'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Switch } from '@/components/ui/switch'
import {
  Dialog, DialogContent, DialogHeader, DialogTitle, DialogDescription, DialogFooter,
} from '@/components/ui/dialog'
import { useSelfHealConfig, useSetSelfHealConfig } from '@/hooks/use-credentials'
import { extractErrorMessage } from '@/lib/utils'
import type { SelfHealConfigPatch } from '@/api/credentials'

interface SelfHealDialogProps {
  open: boolean
  onOpenChange: (open: boolean) => void
}

/** 草稿态用字符串保存数字输入，避免受控 input 在清空时跳回 0 */
interface Draft {
  enabled: boolean
  minIntervalSecs: string
  maxConsecutiveRounds: string
}

const DEFAULT_DRAFT: Draft = {
  enabled: true,
  minIntervalSecs: '300',
  maxConsecutiveRounds: '5',
}

const MAX_INTERVAL_SECS = 86_400
const MAX_ROUNDS = 1_000

/**
 * 凭据自愈设置。
 *
 * 自愈指「当前请求作用域内所有凭据都因连续失败被自动禁用时，恢复它们再试一次」。
 * 三个开关共同约束这个行为，防止持续故障时形成
 * `全禁 → 自愈 → 再失败 → 全禁` 的死循环。
 */
export function SelfHealDialog({ open, onOpenChange }: SelfHealDialogProps) {
  const { data, isLoading } = useSelfHealConfig()
  const { mutate: save, isPending: saving } = useSetSelfHealConfig()

  const [draft, setDraft] = useState<Draft>(DEFAULT_DRAFT)

  useEffect(() => {
    if (data) {
      setDraft({
        enabled: data.enabled,
        minIntervalSecs: String(data.minIntervalSecs),
        maxConsecutiveRounds: String(data.maxConsecutiveRounds),
      })
    }
  }, [data])

  const handleSave = () => {
    const interval = Number(draft.minIntervalSecs)
    const rounds = Number(draft.maxConsecutiveRounds)

    if (!Number.isInteger(interval) || interval < 0 || interval > MAX_INTERVAL_SECS) {
      toast.error(`冷却间隔需为 0-${MAX_INTERVAL_SECS} 之间的整数秒`)
      return
    }
    if (!Number.isInteger(rounds) || rounds < 0 || rounds > MAX_ROUNDS) {
      toast.error(`连续轮数上限需为 0-${MAX_ROUNDS} 之间的整数`)
      return
    }

    const patch: SelfHealConfigPatch = {
      enabled: draft.enabled,
      minIntervalSecs: interval,
      maxConsecutiveRounds: rounds,
    }
    save(patch, {
      onSuccess: (saved) => {
        toast.success(saved.enabled ? '凭据自愈已启用' : '凭据自愈已关闭')
        onOpenChange(false)
      },
      onError: (err) => toast.error(`保存失败: ${extractErrorMessage(err)}`),
    })
  }

  const disabled = isLoading || saving

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-lg">
        <DialogHeader>
          <DialogTitle className="flex items-center gap-2">
            <HeartPulse className="h-4 w-4" />
            凭据自愈
          </DialogTitle>
          <DialogDescription>
            当前请求作用域内所有凭据都因连续失败被自动禁用时，恢复它们再试一次。
            只恢复「连续失败」导致的禁用，手动禁用、额度用尽、Token 失效的凭据不受影响。
          </DialogDescription>
        </DialogHeader>

        <div className="space-y-4">
          <div className="flex items-center justify-between gap-3 rounded-md bg-secondary/40 px-3 py-2.5">
            <div className="text-xs">
              <div className="font-medium text-foreground">
                {draft.enabled ? '已启用' : '已关闭'}
              </div>
              <div className="leading-snug text-muted-foreground">
                {draft.enabled
                  ? '全部凭据被自动禁用时尝试恢复，受下面两项约束'
                  : '全部凭据被自动禁用时直接失败，不做任何恢复尝试'}
              </div>
            </div>
            <Switch
              checked={draft.enabled}
              disabled={disabled}
              onCheckedChange={(enabled) => setDraft((d) => ({ ...d, enabled }))}
            />
          </div>

          <NumberField
            label="最小冷却间隔"
            unit="秒"
            hint="同一凭据两次自愈之间至少间隔这么久。持续故障时把探测频率限制在此节奏，避免每个请求都重置一遍并无效打上游。0 = 不限。"
            value={draft.minIntervalSecs}
            max={MAX_INTERVAL_SECS}
            disabled={disabled || !draft.enabled}
            onChange={(minIntervalSecs) => setDraft((d) => ({ ...d, minIntervalSecs }))}
          />

          <NumberField
            label="连续轮数上限"
            unit="轮"
            hint="同一凭据连续自愈达到此值、期间同一模型上仍无成功调用时，保持禁用并等待人工处理。其它凭据、分组或模型上的成功不会清零该计数。0 = 不限。"
            value={draft.maxConsecutiveRounds}
            max={MAX_ROUNDS}
            disabled={disabled || !draft.enabled}
            onChange={(maxConsecutiveRounds) =>
              setDraft((d) => ({ ...d, maxConsecutiveRounds }))
            }
          />

          {data && <ObservedPanel consecutiveRounds={data.consecutiveRounds} totalCount={data.totalCount} />}
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

function NumberField({
  label, unit, hint, value, max, disabled, onChange,
}: {
  label: string
  unit: string
  hint: string
  value: string
  max: number
  disabled: boolean
  onChange: (value: string) => void
}) {
  return (
    <div className="space-y-1.5">
      <div className="flex items-center gap-2">
        <label className="w-28 shrink-0 text-xs font-medium text-foreground">{label}</label>
        <Input
          type="number"
          min={0}
          max={max}
          value={value}
          disabled={disabled}
          onChange={(e) => onChange(e.target.value)}
          className="h-7 text-xs"
        />
        <span className="shrink-0 text-xs text-muted-foreground">{unit}</span>
      </div>
      <p className="pl-28 text-xs leading-snug text-muted-foreground">{hint}</p>
    </div>
  )
}

/** 服务端返回的只读观测值，帮助判断是否有凭据卡在上限附近 */
function ObservedPanel({
  consecutiveRounds, totalCount,
}: {
  consecutiveRounds: number
  totalCount: number
}) {
  return (
    <div className="grid grid-cols-2 gap-2 border-t pt-3 text-xs">
      <div>
        <div className="text-muted-foreground">当前最大连续轮数</div>
        <div className="font-medium text-foreground">{consecutiveRounds}</div>
      </div>
      <div>
        <div className="text-muted-foreground">累计自愈次数</div>
        <div className="font-medium text-foreground">{totalCount}</div>
      </div>
    </div>
  )
}
