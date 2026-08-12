<script setup lang="ts">
import { computed, shallowRef } from 'vue'
import type { UsageBucket } from '@/api'

const props = defineProps<{ buckets: UsageBucket[] }>()
const mode = shallowRef<'tokens' | 'usd'>('tokens')

const values = computed(() =>
  props.buckets.map((bucket) =>
    mode.value === 'tokens'
      ? bucket.metrics.tokens.total
      : Number(BigInt(bucket.metrics.known_cost_nano_usd)) / 1_000_000_000,
  ),
)

const chartPoints = computed(() => {
  if (values.value.length === 0) return ''
  const max = Math.max(...values.value, 1)
  return values.value
    .map((value, index) => {
      const x = values.value.length === 1 ? 50 : (index / (values.value.length - 1)) * 100
      const y = 92 - (value / max) * 78
      return `${x},${y}`
    })
    .join(' ')
})

const labels = computed(() => {
  if (props.buckets.length <= 3) return props.buckets
  const middle = Math.floor((props.buckets.length - 1) / 2)
  return [props.buckets[0], props.buckets[middle], props.buckets[props.buckets.length - 1]]
})
</script>

<template>
  <section class="space-y-3">
    <div class="flex items-center justify-between gap-3">
      <h2 class="text-sm font-semibold text-[#29261e]">趋势</h2>
      <div class="flex h-8 rounded-md border border-[#d9d2c8] bg-white p-0.5">
        <button
          v-for="item in [{ value: 'tokens', label: 'Token' }, { value: 'usd', label: 'USD' }] as const"
          :key="item.value"
          type="button"
          class="w-16 rounded text-xs transition-colors"
          :class="mode === item.value ? 'bg-[#29261e] text-white' : 'text-[#716a5e] hover:bg-[#f0ebe4]'"
          @click="mode = item.value"
        >
          {{ item.label }}
        </button>
      </div>
    </div>
    <div class="h-64 border-y border-[#e8e2d9] bg-white py-4">
      <svg viewBox="0 0 100 100" preserveAspectRatio="none" class="h-48 w-full" role="img" aria-label="用量趋势">
        <line v-for="y in [14, 40, 66, 92]" :key="y" x1="0" :y1="y" x2="100" :y2="y" stroke="#ebe6de" stroke-width="0.5" />
        <polyline
          v-if="chartPoints"
          :points="chartPoints"
          fill="none"
          stroke="#c4704f"
          stroke-width="2"
          vector-effect="non-scaling-stroke"
        />
      </svg>
      <div class="flex justify-between text-[11px] text-[#8c8475]">
        <span v-for="bucket in labels" :key="bucket.key">{{ bucket.key }}</span>
      </div>
    </div>
  </section>
</template>
