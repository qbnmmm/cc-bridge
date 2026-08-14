<script setup lang="ts">
import { RefreshCw } from 'lucide-vue-next'
import { Button } from '@/components/ui/button'
import type {
  UsageDimensions,
  UsageGranularity,
  UsageGroupBy,
  UsageQueryParams,
} from '@/api'
import type { UsageDatePreset, UsageDateShortcut } from '@/lib/usage'

const props = defineProps<{
  filters: UsageQueryParams
  dimensions: UsageDimensions
  loading: boolean
  datePreset: UsageDatePreset
}>()

const emit = defineEmits<{
  reload: []
  setGranularity: [value: UsageGranularity]
  setGroupBy: [value: UsageGroupBy]
  setDatePreset: [value: UsageDateShortcut]
  updateFilters: [value: Partial<UsageQueryParams>]
}>()

const datePresets: Array<{ value: UsageDateShortcut; label: string }> = [
  { value: 'today', label: '今天' },
  { value: 'this_week', label: '本周' },
  { value: 'this_month', label: '本月' },
  { value: 'last_30_days', label: '近 30 日' },
]

const granularities: Array<{ value: UsageGranularity; label: string }> = [
  { value: 'day', label: '日' },
  { value: 'week', label: '周' },
  { value: 'month', label: '月' },
]

const groups: Array<{ value: UsageGroupBy; label: string }> = [
  { value: 'model', label: '模型' },
  { value: 'account', label: '账号' },
  { value: 'api_token', label: 'API Token' },
]

function inputValue(event: Event): string {
  return (event.target as HTMLInputElement | HTMLSelectElement).value
}

function optionalId(event: Event): number | undefined {
  const value = inputValue(event)
  return value ? Number(value) : undefined
}
</script>

<template>
  <section class="space-y-3 border-y border-[#e8e2d9] py-4">
    <div class="flex flex-wrap items-end justify-between gap-3">
      <div class="flex flex-wrap items-end gap-3">
        <div class="space-y-1">
          <p class="text-xs text-[#716a5e]">查询范围</p>
          <div class="flex h-9 rounded-md border border-[#d9d2c8] bg-white p-1" aria-label="查询范围">
            <button
              v-for="item in datePresets"
              :key="item.value"
              type="button"
              class="min-w-12 rounded px-2 text-sm transition-colors"
              :class="props.datePreset === item.value
                ? 'bg-[#c4704f] text-white'
                : 'text-[#716a5e] hover:bg-[#f0ebe4]'"
              :aria-pressed="props.datePreset === item.value"
              @click="emit('setDatePreset', item.value)"
            >
              {{ item.label }}
            </button>
          </div>
        </div>
        <div class="space-y-1">
          <p class="text-xs text-[#716a5e]">聚合粒度</p>
          <div class="flex h-9 rounded-md border border-[#d9d2c8] bg-white p-1" aria-label="聚合粒度">
            <button
              v-for="item in granularities"
              :key="item.value"
              type="button"
              class="min-w-12 rounded px-3 text-sm transition-colors"
              :class="props.filters.granularity === item.value
                ? 'bg-[#29261e] text-white'
                : 'text-[#716a5e] hover:bg-[#f0ebe4]'"
              :aria-pressed="props.filters.granularity === item.value"
              @click="emit('setGranularity', item.value)"
            >
              {{ item.label }}
            </button>
          </div>
        </div>
      </div>
      <Button
        variant="outline"
        size="sm"
        :disabled="props.loading"
        class="border-[#d9d2c8]"
        @click="emit('reload')"
      >
        <RefreshCw class="size-4" :class="{ 'animate-spin': props.loading }" />
        刷新
      </Button>
    </div>

    <div class="grid gap-3 sm:grid-cols-2 lg:grid-cols-5">
      <label class="space-y-1 text-xs text-[#716a5e]">
        <span>开始日期</span>
        <input
          :value="props.filters.start_date"
          type="date"
          class="usage-control"
          @change="emit('updateFilters', { start_date: inputValue($event) })"
        />
      </label>
      <label class="space-y-1 text-xs text-[#716a5e]">
        <span>结束日期</span>
        <input
          :value="props.filters.end_date"
          type="date"
          class="usage-control"
          @change="emit('updateFilters', { end_date: inputValue($event) })"
        />
      </label>
      <label class="space-y-1 text-xs text-[#716a5e]">
        <span>账号</span>
        <select
          :value="props.filters.account_id ?? ''"
          class="usage-control"
          @change="emit('updateFilters', { account_id: optionalId($event) })"
        >
          <option value="">全部账号</option>
          <option v-for="item in props.dimensions.accounts" :key="item.id" :value="item.id">
            {{ item.label }}
          </option>
        </select>
      </label>
      <label class="space-y-1 text-xs text-[#716a5e]">
        <span>API Token</span>
        <select
          :value="props.filters.api_token_id ?? ''"
          class="usage-control"
          @change="emit('updateFilters', { api_token_id: optionalId($event) })"
        >
          <option value="">全部 Token</option>
          <option v-for="item in props.dimensions.api_tokens" :key="item.id" :value="item.id">
            {{ item.label }}
          </option>
        </select>
      </label>
      <label class="space-y-1 text-xs text-[#716a5e]">
        <span>模型</span>
        <select
          :value="props.filters.model ?? ''"
          class="usage-control"
          @change="emit('updateFilters', { model: inputValue($event) || undefined })"
        >
          <option value="">全部模型</option>
          <option v-for="model in props.dimensions.models" :key="model" :value="model">
            {{ model }}
          </option>
        </select>
      </label>
    </div>

    <div class="flex flex-wrap items-center gap-2">
      <span class="text-xs text-[#716a5e]">拆分</span>
      <button
        v-for="item in groups"
        :key="item.value"
        type="button"
        class="h-8 px-3 text-xs border rounded transition-colors"
        :class="props.filters.group_by === item.value
          ? 'border-[#c4704f] bg-[#c4704f]/10 text-[#a75234]'
          : 'border-[#d9d2c8] bg-white text-[#716a5e] hover:bg-[#f0ebe4]'"
        @click="emit('setGroupBy', item.value)"
      >
        {{ item.label }}
      </button>
    </div>
  </section>
</template>

<style scoped>
.usage-control {
  width: 100%;
  height: 2.25rem;
  border: 1px solid #d9d2c8;
  border-radius: 0.375rem;
  background: white;
  padding: 0 0.625rem;
  color: #29261e;
  font-size: 0.875rem;
}

.usage-control:focus {
  outline: 2px solid rgb(196 112 79 / 25%);
  outline-offset: 1px;
}
</style>
