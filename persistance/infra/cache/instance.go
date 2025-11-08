package cache

import "sync"

var (
	instance *CacheSystem
	once     sync.Once
)

// GetInstance retourne l'instance unique du cache (thread-safe)
func GetInstance() *CacheSystem {
	once.Do(func() {
		instance = NewCache()
	})
	return instance
}
