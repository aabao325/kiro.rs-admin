import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query'
import { getCacheForce, setCacheForce } from '@/api/cache-force'
import type { CacheForceSettings } from '@/types/api'

export function useCacheForce() {
  return useQuery({
    queryKey: ['cache-force'],
    queryFn: getCacheForce,
    staleTime: 5000,
  })
}

export function useSetCacheForce() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: (settings: CacheForceSettings) => setCacheForce(settings),
    onSuccess: (data) => qc.setQueryData(['cache-force'], data),
  })
}
