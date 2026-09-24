<script setup lang="ts">
import { computed } from 'vue'
import type { PerformanceReport } from '../../api'
import { formatMs, formatSpeed, formatTime } from '../../lib/performance'
import { Card, CardContent } from '@/components/ui/card'
const props = defineProps<{ report: PerformanceReport }>()
const metrics = computed(() => [
  { label: '首内容时间', hint: '思考、正文或工具输入开始；排除心跳', stats: props.report.first_content },
  { label: '首正文时间', hint: '首段非空正文；纯工具调用可能没有正文', stats: props.report.first_text },
  { label: '完整耗时', hint: '从网关收到请求到响应流结束', stats: props.report.duration },
])
const trendMax = computed(() => Math.max(1, ...props.report.buckets.flatMap(b => [b.duration.p95 ?? 0, b.first_content.p95 ?? 0])))
const histogramMax = computed(() => Math.max(1, ...props.report.histogram.flatMap(b => [b.duration_count, b.first_content_count])))
const bucketWidth = computed(() => 900 / Math.max(1, props.report.buckets.length))
const errorRate = computed(() => props.report.counts.completed_count ? `${(100 * props.report.counts.error_count / props.report.counts.completed_count).toFixed(1)}%` : '—')
</script>

<template>
  <div class="space-y-4">
    <div class="grid gap-4 md:grid-cols-3">
      <Card v-for="metric in metrics" :key="metric.label" class="border-[#e8e2d9] !gap-0 !py-0">
        <CardContent class="p-5">
          <p class="text-sm font-medium">{{ metric.label }} <span class="float-right text-xs text-[#8c8475]">{{ metric.stats.sample_count }} 样本</span></p>
          <p class="mt-3 text-3xl font-semibold tracking-tight">{{ formatMs(metric.stats.p95) }} <span class="text-xs font-normal text-[#8c8475]">P95</span></p>
          <div class="mt-3 flex flex-wrap gap-x-4 gap-y-1 text-xs text-[#8c8475]"><span>P50 {{ formatMs(metric.stats.p50) }}</span><span>P99 {{ formatMs(metric.stats.p99) }}</span><span>最大 {{ formatMs(metric.stats.max) }}</span></div>
          <p class="mt-3 text-xs text-[#8c8475]">{{ metric.hint }}</p>
        </CardContent>
      </Card>
    </div>
    <div class="grid grid-cols-2 gap-4 rounded-xl border border-[#e8e2d9] bg-white p-5 text-sm md:grid-cols-6">
      <div><p class="text-xs text-[#8c8475]">已结束</p><p class="mt-1 text-xl font-semibold">{{ report.counts.completed_count }}</p></div>
      <div><p class="text-xs text-[#8c8475]">完整成功</p><p class="mt-1 text-xl font-semibold text-emerald-700">{{ report.counts.success_count }}</p></div>
      <div><p class="text-xs text-[#8c8475]">错误 / 已结束</p><p class="mt-1 text-xl font-semibold text-red-700">{{ report.counts.error_count }} <span class="text-xs font-normal">{{ errorRate }}</span></p></div>
      <div><p class="text-xs text-[#8c8475]">取消 / 释放</p><p class="mt-1 text-xl font-semibold">{{ report.counts.aborted_count }}</p></div>
      <div><p class="text-xs text-[#8c8475]">结果未知</p><p class="mt-1 text-xl font-semibold">{{ report.counts.unknown_count }}</p></div>
      <div><p class="text-xs text-[#8c8475]">平均输出速度 · 中位数</p><p class="mt-1 text-xl font-semibold">{{ formatSpeed(report.output_speed.p50) }}</p><p class="text-xs text-[#8c8475]">{{ report.output_speed.sample_count }} 样本，含思考</p></div>
    </div>
    <div class="rounded-xl border border-[#e8e2d9] bg-white p-5">
      <div class="flex flex-wrap justify-between gap-2"><h3 class="font-medium">延迟趋势 · P95</h3><p class="text-xs text-[#8c8475]"><span class="text-[#c4704f]">● 完整耗时</span>　<span class="text-emerald-700">● 首内容</span>　仅完整成功样本</p></div>
      <p v-if="!report.counts.success_count" class="py-12 text-center text-sm text-[#8c8475]">当前范围暂无完整成功样本</p>
      <div v-else class="mt-4">
        <p class="text-xs text-[#8c8475]">刻度上限 {{ formatMs(trendMax) }} · 悬停或聚焦查看每段时间</p>
        <svg viewBox="0 0 900 150" class="mt-2 h-44 w-full" role="img" aria-label="完整耗时和首内容时间的 P95 趋势">
          <g v-for="(bucket, i) in report.buckets" :key="bucket.start_ms" tabindex="0" :aria-label="`${formatTime(bucket.start_ms)}，完整耗时 ${formatMs(bucket.duration.p95)}，首内容 ${formatMs(bucket.first_content.p95)}`">
            <title>{{ formatTime(bucket.start_ms) }} · {{ bucket.completed_count }} 次；完整 P95 {{ formatMs(bucket.duration.p95) }}；首内容 P95 {{ formatMs(bucket.first_content.p95) }}</title>
            <rect :x="i * bucketWidth" y="0" :width="bucketWidth" height="150" fill="transparent" />
            <rect v-if="bucket.duration.p95 != null" :x="i * bucketWidth + bucketWidth * 0.1" :y="145 - (bucket.duration.p95 / trendMax) * 135" :width="Math.max(0.4, bucketWidth * 0.35)" :height="Math.max(1, (bucket.duration.p95 / trendMax) * 135)" rx="1" fill="#c4704f" />
            <rect v-if="bucket.first_content.p95 != null" :x="i * bucketWidth + bucketWidth * 0.55" :y="145 - (bucket.first_content.p95 / trendMax) * 135" :width="Math.max(0.4, bucketWidth * 0.35)" :height="Math.max(1, (bucket.first_content.p95 / trendMax) * 135)" rx="1" fill="#047857" />
          </g>
        </svg>
        <div class="flex justify-between text-xs text-[#8c8475]"><span>{{ formatTime(report.start_ms) }}</span><span>{{ formatTime(report.end_ms) }}</span></div>
      </div>
    </div>
    <div class="rounded-xl border border-[#e8e2d9] bg-white p-5">
      <h3 class="font-medium">延迟分布</h3><p class="mt-1 text-xs text-[#8c8475]">完整耗时 / 首内容，含 30 分钟以上区间</p>
      <div class="mt-4 grid gap-3 sm:grid-cols-2 lg:grid-cols-4">
        <div v-for="bucket in report.histogram" :key="bucket.label" class="text-xs">
          <div class="mb-2 flex justify-between"><span>{{ bucket.label }}</span><span>{{ bucket.duration_count }} / {{ bucket.first_content_count }}</span></div>
          <div class="h-1.5 overflow-hidden rounded bg-[#f4f0e9]"><div class="h-full rounded bg-[#c4704f]" :style="{ width: `${100 * bucket.duration_count / histogramMax}%` }" /></div>
          <div class="mt-1 h-1.5 overflow-hidden rounded bg-[#f4f0e9]"><div class="h-full rounded bg-emerald-700" :style="{ width: `${100 * bucket.first_content_count / histogramMax}%` }" /></div>
        </div>
      </div>
    </div>
  </div>
</template>
