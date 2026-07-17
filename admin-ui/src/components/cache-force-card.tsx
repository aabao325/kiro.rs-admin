import { useEffect, useState } from 'react'
import { Database } from 'lucide-react'
import { toast } from 'sonner'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import {
  Dialog, DialogContent, DialogHeader, DialogTitle, DialogDescription, DialogFooter,
} from '@/components/ui/dialog'
import {
  Select, SelectContent, SelectItem, SelectTrigger, SelectValue,
} from '@/components/ui/select'
import { useCacheForce, useSetCacheForce } from '@/hooks/use-cache-force'
import { extractErrorMessage } from '@/lib/utils'
import type { CacheForceSettings, CacheMode } from '@/types/api'

interface CacheForceDialogProps {
  open: boolean
  onOpenChange: (open: boolean) => void
}

const MODE_LABEL: Record<CacheMode, string> = {
  off: '关闭',
  auto: '智能模拟',
  force: '比例强制',
}

const MODE_DESCRIPTION: Record<CacheMode, string> = {
  off: '完全不注入缓存字段，响应的 cache_creation / cache_read 恒为 0（模拟官方未使用 cache_control 时的响应）。',
  auto: '现状：按请求里 cache_control 断点做哈希链前缀命中模拟，跨轮命中真实存在的前缀才计入缓存。',
  force: '不管请求是否带 cache_control，直接按下面三个比例把本次估算的输入 token 总数强制拆成 input / cache_creation / cache_read。',
}

const DEFAULT_SETTINGS: CacheForceSettings = {
  mode: 'auto',
  creationRatio: 0.25,
  hitRatio: 0.70,
  cacheableRatio: 1.0,
}

/**
 * 缓存强制覆盖：三档模式（关闭 / 智能模拟 / 比例强制）控制响应里
 * cache_creation_input_tokens / cache_read_input_tokens 的生成方式。
 * 比例强制模式下的三个比例仅在该模式生效时才会被使用。
 */
export function CacheForceDialog({ open, onOpenChange }: CacheForceDialogProps) {
  const { data, isLoading } = useCacheForce()
  const { mutate: save, isPending: saving } = useSetCacheForce()

  const [draft, setDraft] = useState<CacheForceSettings>(DEFAULT_SETTINGS)

  useEffect(() => {
    if (data) setDraft(data)
  }, [data])

  const handleSave = () => {
    const creationRatio = clampRatio(draft.creationRatio)
    const hitRatio = clampRatio(draft.hitRatio)
    const cacheableRatio = clampRatio(draft.cacheableRatio)
    save(
      { mode: draft.mode, creationRatio, hitRatio, cacheableRatio },
      {
        onSuccess: (saved) => {
          setDraft(saved)
          toast.success(`已保存缓存模式：${MODE_LABEL[saved.mode]}`)
        },
        onError: (err) => toast.error(`保存失败: ${extractErrorMessage(err)}`),
      },
    )
  }

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-lg">
        <DialogHeader>
          <DialogTitle className="flex items-center gap-2">
            <Database className="h-4 w-4" />
            缓存强制覆盖
          </DialogTitle>
          <DialogDescription>
            控制响应里 <code>cache_creation_input_tokens</code> /{' '}
            <code>cache_read_input_tokens</code> 的生成方式，全局一份设置，对所有 Key 生效。
          </DialogDescription>
        </DialogHeader>

        {isLoading ? (
          <p className="py-6 text-center text-sm text-muted-foreground">加载中…</p>
        ) : (
          <div className="space-y-4 py-1">
            <label className="block text-xs font-medium text-muted-foreground">
              模式
              <Select
                value={draft.mode}
                onValueChange={(value) =>
                  setDraft((prev) => ({ ...prev, mode: value as CacheMode }))
                }
                disabled={saving}
              >
                <SelectTrigger className="mt-1">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  {(['off', 'auto', 'force'] as CacheMode[]).map((mode) => (
                    <SelectItem key={mode} value={mode}>
                      {MODE_LABEL[mode]}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </label>
            <p className="rounded-md bg-secondary/40 px-2.5 py-2 text-xs leading-snug text-muted-foreground">
              {MODE_DESCRIPTION[draft.mode]}
            </p>

            <div className={`grid grid-cols-3 gap-2 ${draft.mode === 'force' ? '' : 'opacity-50'}`}>
              <RatioInput
                label="creationRatio"
                value={draft.creationRatio}
                disabled={saving || draft.mode !== 'force'}
                onChange={(v) => setDraft((prev) => ({ ...prev, creationRatio: v }))}
              />
              <RatioInput
                label="hitRatio"
                value={draft.hitRatio}
                disabled={saving || draft.mode !== 'force'}
                onChange={(v) => setDraft((prev) => ({ ...prev, hitRatio: v }))}
              />
              <RatioInput
                label="cacheableRatio"
                value={draft.cacheableRatio}
                disabled={saving || draft.mode !== 'force'}
                onChange={(v) => setDraft((prev) => ({ ...prev, cacheableRatio: v }))}
              />
            </div>
            <p className="text-[11px] leading-snug text-muted-foreground">
              cacheableRatio：prompt 中算作「可缓存基数」的比例；creationRatio /
              hitRatio：该基数里分别算作 cache_creation / cache_read 的比例（均为 [0,1]，
              超限会自动按比例缩放，input_tokens 恒 ≥ 1）。
            </p>
          </div>
        )}

        <DialogFooter>
          <Button type="button" variant="outline" onClick={() => onOpenChange(false)} disabled={saving}>
            取消
          </Button>
          <Button type="button" onClick={handleSave} disabled={saving || isLoading}>
            {saving ? '保存中…' : '保存'}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}

function clampRatio(value: number) {
  if (!Number.isFinite(value)) return 0
  return Math.min(1, Math.max(0, value))
}

function RatioInput({
  label, value, disabled, onChange,
}: {
  label: string
  value: number
  disabled: boolean
  onChange: (value: number) => void
}) {
  return (
    <label className="text-xs font-medium text-muted-foreground">
      {label}
      <Input
        type="number"
        min={0}
        max={1}
        step={0.05}
        value={value}
        disabled={disabled}
        onChange={(e) => {
          const numeric = Number(e.target.value)
          onChange(Number.isFinite(numeric) ? numeric : 0)
        }}
        className="mt-1 h-8 text-xs"
      />
    </label>
  )
}
