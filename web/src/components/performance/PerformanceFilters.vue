<script setup lang="ts">
import type { PerformanceDimensions } from '../../api'
import type { PerformanceFilters } from '../../composables/usePerformance'
import { Button } from '@/components/ui/button'
const filters = defineModel<PerformanceFilters>({ required: true })
defineProps<{ dimensions: PerformanceDimensions }>()
defineEmits<{ refresh: [] }>()
</script>

<template>
  <div class="flex flex-wrap items-end gap-3 rounded-xl border border-[#e8e2d9] bg-white p-4">
    <label class="space-y-1 text-xs text-[#8c8475]">时间范围（UTC+8）
      <select v-model="filters.range" class="perf-select"><option value="hour">最近 1 小时</option><option value="today">今天</option><option value="day">最近 24 小时</option><option value="week">最近 7 天</option><option value="month">最近 30 个自然日</option></select>
    </label>
    <label class="space-y-1 text-xs text-[#8c8475]">账号
      <select v-model="filters.account" class="perf-select"><option value="">全部账号</option><option v-for="item in dimensions.accounts" :key="item.id" :value="String(item.id)">{{ item.label }}</option></select>
    </label>
    <label class="space-y-1 text-xs text-[#8c8475]">API Token
      <select v-model="filters.token" class="perf-select"><option value="">全部令牌</option><option v-for="item in dimensions.api_tokens" :key="item.id" :value="String(item.id)">{{ item.label }}</option></select>
    </label>
    <label class="space-y-1 text-xs text-[#8c8475]">模型
      <select v-model="filters.model" class="perf-select max-w-64"><option value="">全部模型</option><option v-for="model in dimensions.models" :key="model">{{ model }}</option></select>
    </label>
    <label class="space-y-1 text-xs text-[#8c8475]">实例
      <select v-model="filters.instance" class="perf-select"><option value="">全部历史实例</option><option v-for="id in dimensions.instances" :key="id" :value="id">{{ id.slice(0, 8) }}</option></select>
    </label>
    <Button variant="outline" size="sm" class="ml-auto" @click="$emit('refresh')">刷新数据</Button>
  </div>
</template>
