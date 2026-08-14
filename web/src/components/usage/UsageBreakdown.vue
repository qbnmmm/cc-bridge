<script setup lang="ts">
import type { UsageBreakdownRow, UsageGroupBy } from '@/api'
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from '@/components/ui/table'
import { formatNanoUsd } from '@/lib/usage'

const props = defineProps<{ rows: UsageBreakdownRow[]; groupBy: UsageGroupBy }>()

const groupLabels: Record<UsageGroupBy, string> = {
  account: '账号',
  api_token: 'API Token',
  model: '模型',
}

function formatNumber(value: number): string {
  return new Intl.NumberFormat('zh-CN').format(value)
}

</script>

<template>
  <section class="space-y-3">
    <h2 class="text-sm font-semibold text-[#29261e]">按{{ groupLabels[props.groupBy] }}拆分</h2>
    <div class="overflow-x-auto border-y border-[#e8e2d9]">
      <Table class="min-w-[760px]">
        <TableHeader>
          <TableRow>
            <TableHead>{{ groupLabels[props.groupBy] }}</TableHead>
            <TableHead class="text-right">请求</TableHead>
            <TableHead class="text-right">输入</TableHead>
            <TableHead class="text-right">输出</TableHead>
            <TableHead class="text-right">缓存写入</TableHead>
            <TableHead class="text-right">缓存读取</TableHead>
            <TableHead class="text-right">已知 USD</TableHead>
          </TableRow>
        </TableHeader>
        <TableBody>
          <TableRow v-for="row in props.rows" :key="row.key">
            <TableCell class="max-w-64 truncate font-medium text-[#29261e]" :title="row.label">{{ row.label }}</TableCell>
            <TableCell class="text-right">{{ formatNumber(row.metrics.request_count) }}</TableCell>
            <TableCell class="text-right">{{ formatNumber(row.metrics.tokens.input) }}</TableCell>
            <TableCell class="text-right">{{ formatNumber(row.metrics.tokens.output) }}</TableCell>
            <TableCell class="text-right">
              {{ formatNumber(row.metrics.tokens.cache_creation_5m + row.metrics.tokens.cache_creation_1h) }}
            </TableCell>
            <TableCell class="text-right">{{ formatNumber(row.metrics.tokens.cache_read) }}</TableCell>
            <TableCell class="text-right" :class="{ 'text-amber-700': !row.metrics.cost_complete }">
              {{ formatNanoUsd(row.metrics.known_cost_nano_usd) }}{{ row.metrics.cost_complete ? '' : ' +' }}
            </TableCell>
          </TableRow>
        </TableBody>
      </Table>
    </div>
  </section>
</template>
