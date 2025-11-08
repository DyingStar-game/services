package cache

import "github.com/dgraph-io/ristretto/v2"

type CacheSystem struct {
	cache *ristretto.Cache[string, string]
}

func NewCache() *CacheSystem {
	cache, err := ristretto.NewCache(&ristretto.Config[string, string]{
		NumCounters: 1e7,
		MaxCost:     1 << 30,
		BufferItems: 64,
	})
	if err != nil {
		panic(err)
	}
	cs := CacheSystem{}
	cs.cache = cache
	return &cs
}

func (cs *CacheSystem) Close() { cs.cache.Close() }

func (cs *CacheSystem) Wait() { cs.cache.Wait() }

func (cs *CacheSystem) Set(key string, value string) { cs.cache.Set(key, value, 1) }

func (cs *CacheSystem) Get(key string) (string, bool) { return cs.cache.Get(key) }

func (cs *CacheSystem) Del(key string) { cs.cache.Del(key) }
