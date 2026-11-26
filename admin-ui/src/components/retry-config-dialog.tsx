import { useEffect, useState } from 'react'
import { Repeat } from 'lucide-react'
import { toast } from 'sonner'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Switch } from '@/components/ui/switch'
import {
  Dialog, DialogContent, DialogHeader, DialogTitle, DialogDescription, DialogFooter,
} from '@/components/ui/dialog'
import { useRetryConfig, useSetRetryConfig } from '@/hooks/use-credentials'
import { extractErrorMessage } from '@/lib/utils'
import type { RetryConfigPatch } from '@/api/credentials'

interface RetryConfigDialogProps {
  open: boolean
  onOpenChange: (open: boolean) => void
}

/** 草稿态用字符串保存数字输入，避免受控 input 在清空时跳回 0 */
interface Draft {
  retryOn429: boolean
  maxAttemptsPerCredential: string
  maxTotalAttempts: string
}

const DEFAULT_DRAFT: Draft = {
  retryOn429: true,
  maxAttemptsPerCredential: '3',
  maxTotalAttempts: '4',
}

/** 与服务端 validate_retry_config 保持一致 */
const MIN_ATTEMPTS = 1
const MAX_ATTEMPTS_PER_CREDENTIAL = 10
const MAX_TOTAL_ATTEMPTS = 20

/**
 * 重试策略设置。
 *
 * 单次请求最多打几次上游由两个上限共同决定：
 * `min(作用域内凭据数 × 每凭据尝试次数, 单请求总尝试上限)`，两者都含首次尝试。
 * 429 开关只影响「上游明确限流」这一类，关闭后 429 立即透传给客户端，
 * 408 / 5xx 仍按各自策略重试。
 */
export function RetryConfigDialog({ open, onOpenChange }: RetryConfigDialogProps) {
  const { data, isLoading } = useRetryConfig()
  const { mutate: save, isPending: saving } = useSetRetryConfig()

  const [draft, setDraft] = useState<Draft>(DEFAULT_DRAFT)

  useEffect(() => {
    if (data) {
      setDraft({
        retryOn429: data.retryOn429,
        maxAttemptsPerCredential: String(data.maxAttemptsPerCredential),
        maxTotalAttempts: String(data.maxTotalAttempts),
      })
    }
  }, [data])

  const handleSave = () => {
    const per = Number(draft.maxAttemptsPerCredential)
    const total = Number(draft.maxTotalAttempts)

    if (
      !Number.isInteger(per) ||
      per < MIN_ATTEMPTS ||
      per > MAX_ATTEMPTS_PER_CREDENTIAL
    ) {
      toast.error(`每凭据尝试次数需为 ${MIN_ATTEMPTS}-${MAX_ATTEMPTS_PER_CREDENTIAL} 之间的整数`)
      return
    }
    if (
      !Number.isInteger(total) ||
      total < MIN_ATTEMPTS ||
      total > MAX_TOTAL_ATTEMPTS
    ) {
      toast.error(`单请求总尝试上限需为 ${MIN_ATTEMPTS}-${MAX_TOTAL_ATTEMPTS} 之间的整数`)
      return
    }

    const patch: RetryConfigPatch = {
      retryOn429: draft.retryOn429,
      maxAttemptsPerCredential: per,
      maxTotalAttempts: total,
    }
    save(patch, {
      onSuccess: (saved) => {
        toast.success(
          saved.maxTotalAttempts === 1
            ? '已关闭重试（每个请求只打一次上游）'
            : `重试策略已更新（最多 ${saved.maxTotalAttempts} 次尝试）`,
        )
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
            <Repeat className="h-4 w-4" />
            重试策略
          </DialogTitle>
          <DialogDescription>
            单次请求最多打几次上游，以及被限流时是否在本地退避重试。
            改完立即生效，无需重启。
          </DialogDescription>
        </DialogHeader>

        <div className="space-y-4">
          <div className="flex items-center justify-between gap-3 rounded-md bg-secondary/40 px-3 py-2.5">
            <div className="text-xs">
              <div className="font-medium text-foreground">
                {draft.retryOn429 ? '429 本地重试：开' : '429 本地重试：关'}
              </div>
              <div className="leading-snug text-muted-foreground">
                {draft.retryOn429
                  ? '被限流时按 1s / 2s / 4s 退避重试，总等待可达 7 秒以上'
                  : '429 立即透传给客户端（保留 Retry-After），不再本地等待'}
              </div>
            </div>
            <Switch
              checked={draft.retryOn429}
              disabled={disabled}
              onCheckedChange={(retryOn429) => setDraft((d) => ({ ...d, retryOn429 }))}
            />
          </div>

          <NumberField
            label="每凭据尝试次数"
            unit="次"
            hint={`同一账号最多尝试几次（含首次）。凭据越多，单请求可使用的上限越高，但最终仍受下面的总上限约束。范围 ${MIN_ATTEMPTS}-${MAX_ATTEMPTS_PER_CREDENTIAL}。`}
            value={draft.maxAttemptsPerCredential}
            min={MIN_ATTEMPTS}
            max={MAX_ATTEMPTS_PER_CREDENTIAL}
            disabled={disabled}
            onChange={(maxAttemptsPerCredential) =>
              setDraft((d) => ({ ...d, maxAttemptsPerCredential }))
            }
          />

          <NumberField
            label="单请求总尝试上限"
            unit="次"
            hint={`实际尝试次数 = min(凭据数 × 每凭据尝试次数, 本值)，含首次。设为 1 即完全关闭重试，第一次打不通立刻返回。范围 ${MIN_ATTEMPTS}-${MAX_TOTAL_ATTEMPTS}。`}
            value={draft.maxTotalAttempts}
            min={MIN_ATTEMPTS}
            max={MAX_TOTAL_ATTEMPTS}
            disabled={disabled}
            onChange={(maxTotalAttempts) => setDraft((d) => ({ ...d, maxTotalAttempts }))}
          />

          {data && (
            <div className="border-t pt-3 text-xs text-muted-foreground">
              当前生效：最多 {data.maxTotalAttempts} 次尝试（含首次）
              {data.maxTotalAttempts > 1 ? `，即最多重试 ${data.maxTotalAttempts - 1} 次` : '，不重试'}
              ；429 {data.retryOn429 ? '会本地重试' : '直接透传'}。
            </div>
          )}
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
  label, unit, hint, value, min, max, disabled, onChange,
}: {
  label: string
  unit: string
  hint: string
  value: string
  min: number
  max: number
  disabled: boolean
  onChange: (value: string) => void
}) {
  return (
    <div className="space-y-1.5">
      <div className="flex items-center gap-2">
        <label className="w-32 shrink-0 text-xs font-medium text-foreground">{label}</label>
        <Input
          type="number"
          min={min}
          max={max}
          value={value}
          disabled={disabled}
          onChange={(e) => onChange(e.target.value)}
          className="h-7 text-xs"
        />
        <span className="shrink-0 text-xs text-muted-foreground">{unit}</span>
      </div>
      <p className="pl-32 text-xs leading-snug text-muted-foreground">{hint}</p>
    </div>
  )
}
