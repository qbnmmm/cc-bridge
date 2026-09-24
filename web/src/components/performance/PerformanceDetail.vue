<script setup lang="ts">
import { computed } from 'vue'
import type { PerformanceEvent } from '../../api'
import { formatMs, formatSpeed, formatTime, outcomeLabels, phaseLabels } from '../../lib/performance'
import { Dialog, DialogContent, DialogHeader, DialogTitle, DialogDescription } from '@/components/ui/dialog'
const props = defineProps<{ event: PerformanceEvent | null; error: string }>()
defineEmits<{ close: [] }>()
const stageNames: Record<string, string> = { auth: '鉴权', body_read: '读取请求体', session_hash: '会话识别', routing: '账号调度', slot_acquire: '获取并发槽', rewrite: '请求改写', resolve_token: '上游凭证 / 遥测准备', upstream_headers: '发送上游至响应头', forward_done: '上游转发准备总段' }
const stages = computed(() => Object.entries(props.event?.stages_ms ?? {}).filter(([key]) => key !== 'forward_done').map(([key, value]) => ({ name: stageNames[key] || key, value })))
const milestones = computed(() => props.event ? [
  ['首包', props.event.first_byte_ms], ['首个有效内容', props.event.first_content_ms], ['首正文', props.event.first_text_ms],
  ['模型结束', props.event.model_completed_ms], ['完整耗时', props.event.duration_ms],
] as const : [])
</script>

<template>
  <Dialog :open="event != null" @update:open="open => { if (!open) $emit('close') }">
    <DialogContent class="max-h-[85vh] overflow-y-auto sm:max-w-2xl">
      <DialogHeader><DialogTitle>请求性能详情</DialogTitle><DialogDescription>时间从请求进入网关计算，受网络、压缩缓冲及客户端消费速度影响。</DialogDescription></DialogHeader>
      <template v-if="event">
        <p v-if="error" role="alert" class="text-sm text-red-700">详情刷新失败，显示已有快照：{{ error }}</p>
        <dl class="grid grid-cols-2 gap-3 text-sm">
          <div class="col-span-2"><dt class="text-xs text-[#8c8475]">请求标识</dt><dd class="mt-1 break-all font-mono text-xs">{{ event.request_id }}</dd></div>
          <div><dt class="text-xs text-[#8c8475]">开始时间（UTC+8）</dt><dd>{{ formatTime(event.started_at_utc) }}</dd></div>
          <div><dt class="text-xs text-[#8c8475]">结果 / 阶段</dt><dd>{{ event.outcome ? outcomeLabels[event.outcome] : phaseLabels[event.phase] }}</dd></div>
          <div><dt class="text-xs text-[#8c8475]">请求模型</dt><dd class="break-all">{{ event.request_model || '—' }}</dd></div>
          <div><dt class="text-xs text-[#8c8475]">响应模型</dt><dd class="break-all">{{ event.response_model || '—' }}</dd></div>
          <div><dt class="text-xs text-[#8c8475]">上游 / 下游 HTTP</dt><dd>{{ event.upstream_status ?? '—' }} / {{ event.downstream_status ?? '—' }}</dd></div>
          <div><dt class="text-xs text-[#8c8475]">停止原因</dt><dd>{{ event.stop_reason || '—' }}</dd></div>
          <div><dt class="text-xs text-[#8c8475]">观测质量</dt><dd>{{ event.observation_quality === 'observed' ? '已观测' : event.observation_quality === 'partial' ? '存在无法识别的内容类型' : '内容解析失败' }}</dd></div>
          <div><dt class="text-xs text-[#8c8475]">实例</dt><dd class="break-all font-mono text-xs">{{ event.instance_id }}</dd></div>
        </dl>
        <div class="rounded-lg bg-[#faf8f5] p-4"><h4 class="text-sm font-medium">时间点 · 距请求进入</h4><div v-for="[name, value] in milestones" :key="name" class="mt-2 flex justify-between text-sm"><span>{{ name }}</span><span class="tabular-nums">{{ formatMs(value) }}</span></div></div>
        <div><h4 class="text-sm font-medium">阶段耗时</h4><div v-for="stage in stages" :key="stage.name" class="mt-2 flex justify-between text-sm"><span class="text-[#8c8475]">{{ stage.name }}</span><span>{{ formatMs(stage.value) }}</span></div><p v-if="!stages.length" class="mt-2 text-sm text-[#8c8475]">尚无阶段记录</p></div>
        <div class="grid grid-cols-2 gap-3 border-t border-[#e8e2d9] pt-4 text-sm"><div>最大内容停顿 <strong>{{ formatMs(event.max_content_gap_ms) }}</strong></div><div>平均输出 <strong>{{ formatSpeed(event.output_tokens_per_second) }}</strong></div><div>输出 Token <strong>{{ event.output_tokens ?? '—' }}</strong></div><div>已持续 <strong>{{ formatMs(event.age_ms) }}</strong></div></div>
        <p class="text-xs leading-relaxed text-[#8c8475]">“—”表示未观测到或不适用。非流式响应与一次批量返回无法可靠计算生成速度。取消 / 释放不能精确区分客户端与服务端责任。此记录不包含客户端工具执行或完整 Agent 任务耗时。</p>
      </template>
    </DialogContent>
  </Dialog>
</template>
