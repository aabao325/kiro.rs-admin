import { useEffect, useState } from 'react'
import { ShieldX, Plus, Trash2, ChevronUp, ChevronDown } from 'lucide-react'
import { toast } from 'sonner'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Switch } from '@/components/ui/switch'
import {
  Dialog, DialogContent, DialogHeader, DialogTitle, DialogDescription, DialogFooter,
} from '@/components/ui/dialog'
import {
  Select, SelectContent, SelectItem, SelectTrigger, SelectValue,
} from '@/components/ui/select'
import { useErrorRules, useSetErrorRules } from '@/hooks/use-credentials'
import { extractErrorMessage } from '@/lib/utils'
import type { ErrorRule, RuleAction, RuleMatchMode } from '@/api/credentials'

interface ErrorRulesDialogProps {
  open: boolean
  onOpenChange: (open: boolean) => void
}

const ACTION_LABEL: Record<RuleAction, string> = {
  disable: '禁用凭据',
  cooldown: '临时冷却',
  countFailure: '仅计失败',
  abort: '立即终止',
}

const ACTION_HINT: Record<RuleAction, string> = {
  disable: '立即禁用该凭据并切换到下一个可用凭据，需人工或自愈恢复。',
  cooldown: '让该凭据进入冷却期，到期自动恢复，期间切换到其它凭据。',
  countFailure: '只累加失败计数，达到连续失败阈值后才由既有逻辑禁用。',
  abort: '立即终止本次请求，不重试、不切换、不改动凭据状态。',
}

/** 空白规则模板 */
function blankRule(): ErrorRule {
  return {
    name: '',
    enabled: true,
    keywords: [],
    matchMode: 'any',
    caseSensitive: false,
    statusCodes: [],
    action: 'disable',
    cooldownSecs: 1800,
    selfHealable: false,
    minAvailable: 0,
  }
}

/** 一键预置：覆盖两个最常见场景，避免用户从零摸索关键词 */
const PRESETS: { label: string; hint: string; rule: () => ErrorRule }[] = [
  {
    label: '模型下架',
    hint: '上游返回 400 + Invalid model ID / INVALID_MODEL_ID',
    rule: () => ({
      ...blankRule(),
      name: '模型下架',
      keywords: ['Invalid model ID', 'INVALID_MODEL_ID'],
      statusCodes: [400],
      action: 'disable',
    }),
  },
  {
    label: '账号封禁',
    hint: '上游返回 403 且同时含 suspended 与 locked your account',
    rule: () => ({
      ...blankRule(),
      name: '账号封禁',
      keywords: ['suspended', 'locked your account'],
      matchMode: 'all',
      statusCodes: [403],
      action: 'disable',
    }),
  },
]

/**
 * 自定义错误规则表。
 *
 * 按响应体关键词匹配上游错误并自动处置凭据，用于应对上游临时改报错文案、
 * 下架模型这类「内置判定短语跟不上」的情况。规则顺序即优先级，从上往下求值。
 */
export function ErrorRulesDialog({ open, onOpenChange }: ErrorRulesDialogProps) {
  const { data, isLoading } = useErrorRules()
  const { mutate: save, isPending: saving } = useSetErrorRules()

  const [draft, setDraft] = useState<ErrorRule[]>([])

  useEffect(() => {
    if (data) setDraft(data.rules)
  }, [data])

  const update = (index: number, patch: Partial<ErrorRule>) => {
    setDraft((rules) => rules.map((r, i) => (i === index ? { ...r, ...patch } : r)))
  }

  const remove = (index: number) => {
    setDraft((rules) => rules.filter((_, i) => i !== index))
  }

  const move = (index: number, delta: number) => {
    setDraft((rules) => {
      const next = [...rules]
      const target = index + delta
      if (target < 0 || target >= next.length) return rules
      ;[next[index], next[target]] = [next[target], next[index]]
      return next
    })
  }

  const handleSave = () => {
    const named = draft.filter((r) => r.name.trim() || r.keywords.length > 0)
    const invalid = named.find((r) => r.keywords.length === 0)
    if (invalid) {
      toast.error(`规则「${invalid.name || '未命名'}」没有关键词，不会命中任何响应`)
      return
    }
    save(named, {
      onSuccess: (res) => {
        toast.success(`已保存 ${res.rules.length} 条错误规则`)
        onOpenChange(false)
      },
      onError: (err) => toast.error(`保存失败: ${extractErrorMessage(err)}`),
    })
  }

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-h-[85vh] overflow-y-auto sm:max-w-2xl">
        <DialogHeader>
          <DialogTitle className="flex items-center gap-2">
            <ShieldX className="h-4 w-4" />
            自定义错误规则
          </DialogTitle>
          <DialogDescription>
            按上游响应体的关键词匹配错误并自动处置凭据。用于应对上游改报错文案或下架模型
            —— 内置判定都是固定短语，跟不上变化。规则从上往下求值，顺序即优先级。
          </DialogDescription>
        </DialogHeader>

        <div className="space-y-3">
          {isLoading ? (
            <p className="text-xs text-muted-foreground">加载中…</p>
          ) : draft.length === 0 ? (
            <EmptyState onAdd={(rule) => setDraft([rule])} />
          ) : (
            draft.map((rule, index) => (
              <RuleCard
                key={index}
                rule={rule}
                index={index}
                total={draft.length}
                disabled={saving}
                onChange={(patch) => update(index, patch)}
                onRemove={() => remove(index)}
                onMove={(delta) => move(index, delta)}
              />
            ))
          )}

          {draft.length > 0 && (
            <div className="flex flex-wrap gap-2 border-t pt-3">
              <Button
                variant="outline"
                size="sm"
                disabled={saving}
                onClick={() => setDraft((r) => [...r, blankRule()])}
              >
                <Plus className="h-3.5 w-3.5" />新增规则
              </Button>
              {PRESETS.map((preset) => (
                <Button
                  key={preset.label}
                  variant="outline"
                  size="sm"
                  disabled={saving}
                  title={preset.hint}
                  onClick={() => setDraft((r) => [...r, preset.rule()])}
                >
                  预置：{preset.label}
                </Button>
              ))}
            </div>
          )}
        </div>

        <DialogFooter>
          <Button variant="outline" size="sm" disabled={saving} onClick={() => onOpenChange(false)}>
            取消
          </Button>
          <Button size="sm" disabled={isLoading || saving} onClick={handleSave}>
            {saving ? '保存中…' : '保存'}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}

function RuleCard({
  rule, index, total, disabled, onChange, onRemove, onMove,
}: {
  rule: ErrorRule
  index: number
  total: number
  disabled: boolean
  onChange: (patch: Partial<ErrorRule>) => void
  onRemove: () => void
  onMove: (delta: number) => void
}) {
  return (
    <div className="space-y-2.5 rounded-md border px-3 py-2.5">
      <div className="flex items-center gap-2">
        <span className="shrink-0 text-xs text-muted-foreground">#{index + 1}</span>
        <Input
          placeholder="规则名（如：模型下架）"
          value={rule.name}
          disabled={disabled}
          onChange={(e) => onChange({ name: e.target.value })}
          className="h-7 text-xs"
        />
        <Switch
          checked={rule.enabled}
          disabled={disabled}
          onCheckedChange={(enabled) => onChange({ enabled })}
          title={rule.enabled ? '已启用' : '已停用'}
        />
        <Button
          variant="ghost"
          size="icon"
          className="h-7 w-7"
          disabled={disabled || index === 0}
          title="上移（提高优先级）"
          onClick={() => onMove(-1)}
        >
          <ChevronUp className="h-3.5 w-3.5" />
        </Button>
        <Button
          variant="ghost"
          size="icon"
          className="h-7 w-7"
          disabled={disabled || index === total - 1}
          title="下移"
          onClick={() => onMove(1)}
        >
          <ChevronDown className="h-3.5 w-3.5" />
        </Button>
        <Button
          variant="ghost"
          size="icon"
          className="h-7 w-7 text-destructive"
          disabled={disabled}
          title="删除该规则"
          onClick={onRemove}
        >
          <Trash2 className="h-3.5 w-3.5" />
        </Button>
      </div>

      <Field label="关键词">
        <Input
          placeholder="多个关键词用英文逗号分隔"
          value={rule.keywords.join(', ')}
          disabled={disabled}
          onChange={(e) =>
            onChange({
              keywords: e.target.value
                .split(',')
                .map((k) => k.trim())
                .filter(Boolean),
            })
          }
          className="h-7 text-xs"
        />
      </Field>

      <div className="grid grid-cols-2 gap-2">
        <Field label="组合方式">
          <Select
            value={rule.matchMode}
            disabled={disabled}
            onValueChange={(v) => onChange({ matchMode: v as RuleMatchMode })}
          >
            <SelectTrigger className="h-7 text-xs">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="any">任一命中</SelectItem>
              <SelectItem value="all">全部命中</SelectItem>
            </SelectContent>
          </Select>
        </Field>
        <Field label="状态码">
          <Input
            placeholder="如 400（留空=不限）"
            value={rule.statusCodes.join(', ')}
            disabled={disabled}
            onChange={(e) =>
              onChange({
                statusCodes: e.target.value
                  .split(',')
                  .map((s) => parseInt(s.trim(), 10))
                  .filter((n) => Number.isInteger(n) && n >= 100 && n <= 599),
              })
            }
            className="h-7 text-xs"
          />
        </Field>
      </div>

      <Field label="命中动作">
        <Select
          value={rule.action}
          disabled={disabled}
          onValueChange={(v) => onChange({ action: v as RuleAction })}
        >
          <SelectTrigger className="h-7 text-xs">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            {(Object.keys(ACTION_LABEL) as RuleAction[]).map((a) => (
              <SelectItem key={a} value={a}>
                {ACTION_LABEL[a]}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
      </Field>
      <p className="text-xs leading-snug text-muted-foreground">{ACTION_HINT[rule.action]}</p>

      {rule.action === 'cooldown' && (
        <Field label="冷却秒数">
          <Input
            type="number"
            min={1}
            max={86400}
            value={rule.cooldownSecs}
            disabled={disabled}
            onChange={(e) => onChange({ cooldownSecs: Number(e.target.value) })}
            className="h-7 text-xs"
          />
        </Field>
      )}

      {rule.action === 'disable' && (
        <>
          <Field label="保底可用数">
            <Input
              type="number"
              min={0}
              value={rule.minAvailable}
              disabled={disabled}
              onChange={(e) => onChange({ minAvailable: Number(e.target.value) })}
              className="h-7 text-xs"
            />
          </Field>
          <p className="text-xs leading-snug text-muted-foreground">
            0 = 无防护，命中就禁用。设为 1 以上时，若禁用会让可用凭据少于该值，则降级为
            仅计失败。用于避免「模型下架」这类根因不在账号的错误把整个凭据池逐个禁干净。
          </p>
        </>
      )}

      <div className="flex flex-wrap items-center gap-x-4 gap-y-2 pt-0.5">
        <Toggle
          label="区分大小写"
          checked={rule.caseSensitive}
          disabled={disabled}
          onChange={(caseSensitive) => onChange({ caseSensitive })}
        />
        <Toggle
          label="允许自愈恢复"
          checked={rule.selfHealable}
          disabled={disabled}
          onChange={(selfHealable) => onChange({ selfHealable })}
        />
      </div>
    </div>
  )
}

function Field({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div className="flex items-center gap-2">
      <label className="w-20 shrink-0 text-xs text-muted-foreground">{label}</label>
      <div className="flex-1">{children}</div>
    </div>
  )
}

function Toggle({
  label, checked, disabled, onChange,
}: {
  label: string
  checked: boolean
  disabled: boolean
  onChange: (checked: boolean) => void
}) {
  return (
    <div className="flex items-center gap-1.5">
      <Switch checked={checked} disabled={disabled} onCheckedChange={onChange} />
      <span className="text-xs text-muted-foreground">{label}</span>
    </div>
  )
}

function EmptyState({ onAdd }: { onAdd: (rule: ErrorRule) => void }) {
  return (
    <div className="space-y-3 rounded-md bg-secondary/40 px-3 py-4 text-xs">
      <p className="text-muted-foreground">
        当前没有任何规则，上游错误完全按内置逻辑处理（与不启用本功能等价）。
        可以从预置开始，或新建一条空白规则。
      </p>
      <div className="flex flex-wrap gap-2">
        {PRESETS.map((preset) => (
          <Button
            key={preset.label}
            variant="outline"
            size="sm"
            title={preset.hint}
            onClick={() => onAdd(preset.rule())}
          >
            预置：{preset.label}
          </Button>
        ))}
        <Button variant="outline" size="sm" onClick={() => onAdd(blankRule())}>
          <Plus className="h-3.5 w-3.5" />空白规则
        </Button>
      </div>
    </div>
  )
}
