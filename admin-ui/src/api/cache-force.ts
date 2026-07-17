import axios from 'axios'
import { storage } from '@/lib/storage'
import type { CacheForceSettings } from '@/types/api'

const api = axios.create({
  baseURL: '/api/admin',
  timeout: 15000,
  headers: { 'Content-Type': 'application/json' },
})

api.interceptors.request.use((config) => {
  const apiKey = storage.getApiKey()
  if (apiKey) config.headers['x-api-key'] = apiKey
  return config
})

export async function getCacheForce(): Promise<CacheForceSettings> {
  const { data } = await api.get<CacheForceSettings>('/cache-force')
  return data
}

export async function setCacheForce(
  settings: CacheForceSettings,
): Promise<CacheForceSettings> {
  const { data } = await api.put<CacheForceSettings>('/cache-force', settings)
  return data
}
