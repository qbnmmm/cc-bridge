<script setup lang="ts">
import { computed } from 'vue'
import type { PerformanceDimensions, PerformanceEvent, PerformancePage } from '../../api'
import { formatMs, formatSpeed, formatTime, outcomeLabels, phaseLabels } from '../../lib/performance'
import { Button } from '@/components/ui/button'
const props = defineProps<{ data: PerformancePage | null; active?: boolean; loading: boolean; dimensions: PerformanceDimensions }>()
const emit = defineEmits<{ page: [value: number]; select: [event: PerformanceEvent] }>()
const accountNames = computed(() => new Map(props.dimensions.accounts.map(a => [a.id, a.label])))
const tokenNames = computed(() => new Map(props.dimensions.api_tokens.map(a => [a.id, a.label])))
const pages = computed(() => props.data ? Math.max(1, Math.ceil(props.data.total / props.data.page_size)) : 1)
</script>

<template>
  <div class="overflow-hidden rounded-xl border border-[#e8e2d9] bg-white">
    <div class="request-scroll overflow-x-auto">
      <table class="w-full min-w-[940px] text-left text-sm">
        <thead class="border-b border-[#e8e2d9] bg-[#faf8f5] text-xs text-[#8c8475]"><tr><th class="p-4 font-medium">请求 / 开始时间</th><th class="p-4 font-medium">模型 / 账号</th><th class="p-4 font-medium">{{ active ? '当前阶段' : '结果' }}</th><th class="p-4 font-medium">{{ active ? '已持续' : '完整耗时' }}</th><th class="p-4 font-medium">首内容 / 首正文</th><th class="p-4 font-medium">{{ active ? '无内容进展' : '最大停顿 / 输出速度' }}</th><th class="p-4"><span class="sr-only">详情</span></th></tr></thead>
        <tbody>
          <tr v-for="event in data?.items" :key="event.request_id" class="border-b border-[#f0ebe4] last:border-0 hover:bg-[#faf8f5]">
            <td class="p-4"><p class="font-mono text-xs">{{ event.request_id.slice(0, 8) }}</p><p class="mt-1 whitespace-nowrap text-xs text-[#8c8475]">{{ formatTime(event.started_at_utc) }}</p><p class="mt-1 text-xs text-[#8c8475]">{{ event.api_token_id == null ? '未识别令牌' : tokenNames.get(event.api_token_id) || `Token #${event.api_token_id}` }}</p></td>
            <td class="p-4"><p class="max-w-56 truncate" :title="event.request_model || ''">{{ event.request_model || '模型未知' }}</p><p class="mt-1 text-xs text-[#8c8475]">{{ event.account_id == null ? '未分配账号' : accountNames.get(event.account_id) || `账号 #${event.account_id}` }}</p></td>
            <td class="p-4"><span class="rounded-md bg-[#f4f0e9] px-2 py-1 text-xs" :class="{ 'text-red-700': event.outcome && !['success', 'aborted', 'unknown'].includes(event.outcome), 'text-emerald-700': event.outcome === 'success' }">{{ active ? phaseLabels[event.phase] || event.phase : event.outcome ? outcomeLabels[event.outcome] : '进行中' }}</span><p class="mt-2 text-xs text-[#8c8475]">{{ event.request_model == null && event.upstream_status == null ? '响应方式待识别' : event.is_stream ? '流式' : '非流式' }}<span v-if="event.downstream_status"> · {{ event.downstream_status }}</span></p><p v-if="event.observation_quality !== 'observed'" class="mt-1 text-xs text-amber-700">内容观测不完整</p></td>
            <td class="whitespace-nowrap p-4 font-medium" :class="{ 'text-amber-700': (event.duration_ms ?? event.age_ms) >= 300000 }">{{ formatMs(active ? event.age_ms : event.duration_ms) }}</td>
            <td class="whitespace-nowrap p-4"><p>{{ formatMs(event.first_content_ms) }}</p><p class="mt-1 text-xs text-[#8c8475]">{{ event.first_text_ms == null ? '未观测到正文' : formatMs(event.first_text_ms) }}</p></td>
            <td class="whitespace-nowrap p-4"><template v-if="active"><p :class="{ 'text-amber-700': event.content_idle_ms >= 60000 && event.model_completed_ms == null }">{{ event.model_completed_ms == null ? formatMs(event.content_idle_ms) : '模型已结束' }}</p><p class="mt-1 text-xs text-[#8c8475]">{{ event.first_content_ms == null ? '尚无有效内容' : '心跳不计作进展' }}</p></template><template v-else><p>{{ formatMs(event.max_content_gap_ms) }}</p><p class="mt-1 text-xs text-[#8c8475]">{{ formatSpeed(event.output_tokens_per_second) }}</p></template></td>
            <td class="p-4"><Button variant="ghost" size="sm" @click="emit('select', event)">详情</Button></td>
          </tr>
          <tr v-if="!data?.items.length"><td colspan="7" class="p-10 text-center text-[#8c8475]">{{ loading ? '正在加载请求…' : active ? '当前实例没有符合条件的进行中请求' : '当前范围没有请求记录' }}</td></tr>
        </tbody>
      </table>
    </div>
    <div v-if="data" class="flex items-center justify-between border-t border-[#e8e2d9] px-4 py-3 text-xs text-[#8c8475]"><span>共 {{ data.total }} 条 · 第 {{ data.page }} / {{ pages }} 页</span><div class="flex gap-2"><Button variant="outline" size="sm" :disabled="data.page <= 1 || loading" @click="emit('page', data.page - 1)">上一页</Button><Button variant="outline" size="sm" :disabled="data.page >= pages || loading" @click="emit('page', data.page + 1)">下一页</Button></div></div>
  </div>
</template>

<style scoped>
.request-scroll { contain: layout inline-size; }
</style>
