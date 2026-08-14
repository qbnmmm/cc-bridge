<script setup lang="ts">
import { AlertTriangle, CircleDollarSign, Database, MessagesSquare } from 'lucide-vue-next'
import type { UsageMetrics } from '@/api'
import { formatNanoUsd } from '@/lib/usage'

const props = defineProps<{ metrics: UsageMetrics }>()

function formatNumber(value: number): string {
  return new Intl.NumberFormat('zh-CN').format(value)
}

</script>

<template>
  <section class="space-y-3">
    <div class="grid grid-cols-2 gap-3 lg:grid-cols-4">
      <article class="summary-cell">
        <MessagesSquare class="size-4 text-[#c4704f]" />
        <p class="summary-label">请求</p>
        <p class="summary-value">{{ formatNumber(props.metrics.request_count) }}</p>
      </article>
      <article class="summary-cell">
        <Database class="size-4 text-emerald-600" />
        <p class="summary-label">总 Token</p>
        <p class="summary-value">{{ formatNumber(props.metrics.tokens.total) }}</p>
      </article>
      <article class="summary-cell">
        <CircleDollarSign class="size-4 text-sky-600" />
        <p class="summary-label">{{ props.metrics.cost_complete ? '成本' : '已知成本' }}</p>
        <p class="summary-value">{{ formatNanoUsd(props.metrics.known_cost_nano_usd) }}</p>
      </article>
      <article class="summary-cell">
        <AlertTriangle class="size-4" :class="props.metrics.cost_complete ? 'text-[#a8a298]' : 'text-amber-600'" />
        <p class="summary-label">未定价 Token</p>
        <p class="summary-value">{{ formatNumber(props.metrics.unpriced_tokens) }}</p>
      </article>
    </div>
    <div class="grid grid-cols-2 gap-x-4 gap-y-2 border-y border-[#e8e2d9] py-3 sm:grid-cols-5">
      <div v-for="item in [
        { label: '输入', value: props.metrics.tokens.input },
        { label: '输出', value: props.metrics.tokens.output },
        { label: 'Cache 写入 5m', value: props.metrics.tokens.cache_creation_5m },
        { label: 'Cache 写入 1h', value: props.metrics.tokens.cache_creation_1h },
        { label: 'Cache 读取', value: props.metrics.tokens.cache_read },
      ]" :key="item.label" class="min-w-0">
        <p class="text-[11px] text-[#8c8475]">{{ item.label }}</p>
        <p class="truncate text-sm font-semibold text-[#4d483f]" :title="formatNumber(item.value)">
          {{ formatNumber(item.value) }}
        </p>
      </div>
    </div>
  </section>
</template>

<style scoped>
.summary-cell {
  display: grid;
  min-height: 6.75rem;
  grid-template-columns: auto 1fr;
  align-content: center;
  gap: 0.25rem 0.5rem;
  border-bottom: 2px solid #e8e2d9;
  padding: 0.75rem 0;
}

.summary-label {
  color: #716a5e;
  font-size: 0.75rem;
}

.summary-value {
  grid-column: 1 / -1;
  color: #29261e;
  font-size: 1.5rem;
  font-weight: 700;
}
</style>
