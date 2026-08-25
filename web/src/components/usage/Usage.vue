<script setup lang="ts">
import { AlertCircle, AlertTriangle, BarChart3 } from 'lucide-vue-next'
import { Button } from '@/components/ui/button'
import { useUsageReport } from '@/composables/useUsageReport'
import UsageBreakdown from './UsageBreakdown.vue'
import UsageFilters from './UsageFilters.vue'
import UsageSummary from './UsageSummary.vue'
import UsageTrend from './UsageTrend.vue'

const {
  filters,
  report,
  trendBuckets,
  dimensions,
  loading,
  error,
  hasIngestionGap,
  datePreset,
  loadReport,
  applyFilters,
  setGranularity,
  setGroupBy,
  setDatePreset,
} = useUsageReport()
</script>

<template>
  <div class="space-y-6">
    <div class="flex items-end justify-between gap-4">
      <div>
        <h1 class="text-xl font-semibold text-[#29261e]">用量</h1>
        <p class="mt-1 text-sm text-[#716a5e]">Asia/Singapore · 最多保留 365 日</p>
      </div>
    </div>

    <UsageFilters
      :filters="filters"
      :dimensions="dimensions"
      :loading="loading"
      :date-preset="datePreset"
      @reload="loadReport"
      @set-date-preset="setDatePreset"
      @set-granularity="setGranularity"
      @set-group-by="setGroupBy"
      @update-filters="applyFilters"
    />

    <div v-if="error" class="flex flex-wrap items-center justify-between gap-3 border border-red-200 bg-red-50 px-4 py-3 text-sm text-red-800">
      <span class="flex items-center gap-2"><AlertCircle class="size-4" />{{ error }}</span>
      <Button variant="outline" size="sm" class="border-red-300" @click="loadReport">重试</Button>
    </div>

    <div v-if="hasIngestionGap && report" class="flex items-start gap-2 border border-amber-200 bg-amber-50 px-4 py-3 text-sm text-amber-900">
      <AlertTriangle class="mt-0.5 size-4 shrink-0" />
      <span>
        本进程检测到统计缺口：队列丢弃 {{ report.ingestion.queue_dropped_total }}，写入失败
        {{ report.ingestion.write_failed_total }}，解析失败
        {{ report.ingestion.parse_failed_total + report.ingestion.parse_oversize_total }}。
      </span>
    </div>

    <div v-if="loading && !report" class="grid grid-cols-2 gap-3 lg:grid-cols-4" aria-label="正在加载用量">
      <div v-for="index in 4" :key="index" class="h-28 animate-pulse border-b-2 border-[#e8e2d9] bg-[#f5f2ed]" />
    </div>

    <template v-else-if="report">
      <UsageSummary :metrics="report.summary" />
      <div v-if="report.summary.request_count === 0" class="flex min-h-64 flex-col items-center justify-center border-y border-[#e8e2d9] text-center">
        <BarChart3 class="mb-3 size-8 text-[#b5b0a6]" />
        <p class="text-sm font-medium text-[#4d483f]">所选范围暂无用量</p>
      </div>
      <template v-else>
        <UsageTrend :buckets="trendBuckets" />
        <UsageBreakdown :rows="report.breakdown" :group-by="filters.group_by" />
      </template>
    </template>
  </div>
</template>
