<script setup lang="ts">
import { computed, ref } from 'vue'
import { usePerformance } from '../../composables/usePerformance'
import { formatTime, outcomeLabels } from '../../lib/performance'
import PerformanceFilters from './PerformanceFilters.vue'
import PerformanceOverview from './PerformanceOverview.vue'
import PerformanceRequests from './PerformanceRequests.vue'
import PerformanceDetail from './PerformanceDetail.vue'
import { Button } from '@/components/ui/button'
const { filters, historyFilters, page, activePage, overview, active, history, dimensions, selected, errors, loading, updated, refresh, loadHistory, showDetail, closeDetail } = usePerformance()
const tab = ref<'active' | 'history'>('active')
const health = computed(() => active.value?.health ?? overview.value?.health)
const hasGaps = computed(() => health.value && (health.value.dropped_total + health.value.active_tracking_dropped + health.value.parse_failed_total > 0))
</script>

<template>
  <section class="space-y-5 text-[#29261e]">
    <div><h2 class="text-2xl font-semibold tracking-tight">请求性能</h2><p class="mt-1 text-sm text-[#8c8475]">定位首内容等待、生成停顿和长请求。历史保留 30 天，网关之外的工具执行不计入。</p></div>
    <PerformanceFilters v-model="filters" :dimensions="dimensions" @refresh="refresh" />
    <p v-if="errors.dimensions" role="alert" class="text-sm text-red-700">筛选项加载失败：{{ errors.dimensions }}</p>
    <div v-if="health && !health.enabled" class="rounded-lg border border-amber-200 bg-amber-50 p-3 text-sm text-amber-900">性能采集已关闭，当前展示已保存的历史。</div>
    <div v-if="hasGaps && health" role="status" class="rounded-lg border border-amber-200 bg-amber-50 p-3 text-sm text-amber-900">自 {{ formatTime(health.since_utc) }} 起存在观测缺口：{{ health.dropped_total }} 条记录丢弃，{{ health.active_tracking_dropped }} 条未进入实时列表，{{ health.parse_failed_total }} 条内容解析失败。统计可能不完整。</div>
    <p v-if="errors.overview" role="alert" class="text-sm text-red-700">概览刷新失败，保留最近结果：{{ errors.overview }}</p>
    <PerformanceOverview v-if="overview" :report="overview" />
    <div v-else class="rounded-xl border border-[#e8e2d9] bg-white p-10 text-center text-[#8c8475]">{{ loading.overview ? '正在加载性能概览…' : '性能概览暂不可用' }}</div>
    <p v-if="updated.overview" class="text-right text-xs text-[#8c8475]">概览更新于 {{ formatTime(updated.overview) }} · 每 60 秒刷新 · 延迟仅统计完整成功请求</p>
    <div class="flex flex-wrap items-center justify-between gap-3">
      <div class="flex gap-2" role="tablist" aria-label="请求类型"><Button :variant="tab === 'active' ? 'default' : 'outline'" role="tab" :aria-selected="tab === 'active'" @click="tab = 'active'">进行中 · 当前实例<span v-if="active">（{{ active.total }}）</span></Button><Button :variant="tab === 'history' ? 'default' : 'outline'" role="tab" :aria-selected="tab === 'history'" @click="tab = 'history'">慢请求明细</Button></div>
      <p v-if="tab === 'active' && active" class="text-xs text-[#8c8475]">实例 {{ active.instance_id.slice(0, 8) }} · 每 5 秒刷新 · {{ formatTime(active.generated_at_utc) }}</p>
    </div>
    <template v-if="tab === 'active'">
      <p class="text-xs text-[#8c8475]">按已持续时间排序，包含所选时间范围之前开始的请求。“无内容进展”忽略心跳；思考和工具输入算作进展。重启前的活跃请求不保留。</p>
      <p v-if="errors.active" role="alert" class="text-sm text-red-700">实时列表刷新失败，显示上次快照：{{ errors.active }}</p>
      <PerformanceRequests :data="active" active :loading="loading.active" :dimensions="dimensions" @page="activePage = $event" @select="showDetail" />
    </template>
    <template v-else>
      <div class="flex flex-wrap items-end gap-3">
        <label class="text-xs text-[#8c8475]">结果<select v-model="historyFilters.outcome" class="perf-select"><option value="">全部结果</option><option v-for="(label, key) in outcomeLabels" :key="key" :value="key">{{ label }}</option></select></label>
        <label class="text-xs text-[#8c8475]">响应方式<select v-model="historyFilters.stream" class="perf-select"><option value="">全部</option><option value="true">流式</option><option value="false">非流式</option></select></label>
        <label class="text-xs text-[#8c8475]">至少耗时（秒）<input v-model="historyFilters.minSeconds" type="number" min="0" step="1" placeholder="不限" class="perf-select w-32" /></label>
        <label class="text-xs text-[#8c8475]">排序<select v-model="historyFilters.sort" class="perf-select"><option value="duration_desc">最慢优先</option><option value="created_at_desc">最新优先</option></select></label>
        <Button variant="outline" size="sm" @click="loadHistory">刷新明细</Button><span v-if="updated.history" class="text-xs text-[#8c8475]">{{ formatTime(updated.history) }}</span>
      </div>
      <p v-if="errors.history" role="alert" class="text-sm text-red-700">历史查询失败，保留最近结果：{{ errors.history }}</p>
      <PerformanceRequests :data="history" :loading="loading.history" :dimensions="dimensions" @page="page = $event" @select="showDetail" />
    </template>
    <PerformanceDetail :event="selected" :error="errors.detail" @close="closeDetail" />
  </section>
</template>

<style scoped>
:deep(.perf-select) { display: block; margin-top: 0.35rem; border: 1px solid #e8e2d9; border-radius: 0.5rem; background: white; padding: 0.5rem 0.7rem; font-size: 0.875rem; color: #29261e; }
:deep(.perf-select:focus) { outline: 2px solid #c4704f80; outline-offset: 1px; }
</style>
