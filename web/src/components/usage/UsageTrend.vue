<script setup lang="ts">
import { computed, shallowRef, watch } from 'vue'
import type { UsageBucket } from '@/api'
import { formatNanoUsd, nanoUsdToUsd } from '@/lib/usage'

const props = defineProps<{ buckets: UsageBucket[] }>()
const mode = shallowRef<'tokens' | 'usd'>('tokens')
const activeIndex = shallowRef<number | null>(null)

const values = computed(() =>
  props.buckets.map((bucket) =>
    mode.value === 'tokens'
      ? bucket.metrics.tokens.total
      : nanoUsdToUsd(bucket.metrics.known_cost_nano_usd),
  ),
)

const chartPoints = computed(() => {
  if (values.value.length === 0) return []
  const max = Math.max(...values.value, 1)
  return values.value
    .map((value, index) => ({
      bucket: props.buckets[index],
      index,
      value,
      x: values.value.length === 1 ? 50 : 2 + (index / (values.value.length - 1)) * 96,
      y: 92 - (value / max) * 78,
    }))
})

const polylinePoints = computed(() =>
  chartPoints.value
    .map((point) => `${point.x},${point.y}`)
    .join(' '),
)

const activePoint = computed(() => {
  if (activeIndex.value === null) return null
  return chartPoints.value[activeIndex.value] ?? null
})

watch(
  () => props.buckets,
  () => {
    activeIndex.value = null
  },
)

function bucketLabel(bucket: UsageBucket): string {
  return bucket.start_date === bucket.end_date
    ? bucket.start_date
    : `${bucket.start_date} 至 ${bucket.end_date}`
}

function formatNumber(value: number): string {
  return new Intl.NumberFormat('zh-CN').format(value)
}

function pointAriaLabel(bucket: UsageBucket): string {
  const costLabel = bucket.metrics.cost_complete ? 'USD' : '已知 USD'
  return `${bucketLabel(bucket)}，Token ${formatNumber(bucket.metrics.tokens.total)}，${costLabel} ${formatNanoUsd(bucket.metrics.known_cost_nano_usd)}`
}

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
          :aria-pressed="mode === item.value"
          @click="mode = item.value"
        >
          {{ item.label }}
        </button>
      </div>
    </div>
    <div class="h-64 border-y border-[#e8e2d9] bg-white py-4">
      <div class="relative h-48" @mouseleave="activeIndex = null">
        <svg viewBox="0 0 100 100" preserveAspectRatio="none" class="pointer-events-none h-full w-full" role="img" aria-label="用量趋势">
          <line v-for="y in [14, 40, 66, 92]" :key="y" x1="0" :y1="y" x2="100" :y2="y" stroke="#ebe6de" stroke-width="0.5" />
          <polyline
            v-if="polylinePoints"
            :points="polylinePoints"
            fill="none"
            stroke="#c4704f"
            stroke-width="2"
            vector-effect="non-scaling-stroke"
          />
        </svg>
        <button
          v-for="point in chartPoints"
          :key="point.bucket.key"
          type="button"
          class="absolute size-5 -translate-x-1/2 -translate-y-1/2 cursor-crosshair rounded-full border-0 bg-transparent p-0 outline-none focus-visible:ring-2 focus-visible:ring-[#c4704f]/30"
          :style="{ left: `${point.x}%`, top: `${point.y}%` }"
          :aria-label="pointAriaLabel(point.bucket)"
          @mouseenter="activeIndex = point.index"
          @focus="activeIndex = point.index"
          @blur="activeIndex = null"
          @click="activeIndex = point.index"
        >
          <span
            class="absolute left-1/2 top-1/2 -translate-x-1/2 -translate-y-1/2 rounded-full border-[1.5px] border-[#c4704f] bg-white transition-[width,height]"
            :class="activeIndex === point.index ? 'size-3' : 'size-2.5'"
          />
        </button>
        <div
          v-if="activePoint"
          class="pointer-events-none absolute z-10 w-44 border border-[#d9d2c8] bg-white px-3 py-2 text-xs shadow-lg"
          :style="{
            left: `clamp(5.5rem, ${activePoint.x}%, calc(100% - 5.5rem))`,
            top: `${activePoint.y}%`,
            transform: activePoint.y < 35
              ? 'translate(-50%, 12px)'
              : 'translate(-50%, calc(-100% - 12px))',
          }"
        >
          <p class="font-medium text-[#29261e]">{{ bucketLabel(activePoint.bucket) }}</p>
          <div class="mt-1.5 flex items-center justify-between gap-3 text-[#716a5e]">
            <span>Token</span>
            <span class="font-medium text-[#29261e]">{{ formatNumber(activePoint.bucket.metrics.tokens.total) }}</span>
          </div>
          <div class="mt-1 flex items-center justify-between gap-3 text-[#716a5e]">
            <span>{{ activePoint.bucket.metrics.cost_complete ? 'USD' : '已知 USD' }}</span>
            <span
              class="font-medium"
              :class="activePoint.bucket.metrics.cost_complete ? 'text-[#29261e]' : 'text-amber-700'"
            >
              {{ formatNanoUsd(activePoint.bucket.metrics.known_cost_nano_usd) }}
            </span>
          </div>
        </div>
      </div>
      <div class="flex justify-between text-[11px] text-[#8c8475]">
        <span v-for="bucket in labels" :key="bucket.key">{{ bucket.key }}</span>
      </div>
    </div>
  </section>
</template>
