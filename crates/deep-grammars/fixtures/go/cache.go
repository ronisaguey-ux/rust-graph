package cache

import (
	"sync"
	"time"
)

// Cache is a generic, concurrency-safe cache with expiry.
type Cache[K comparable, V any] struct {
	mu    sync.Mutex
	items map[K]entry[V]
	ttl   time.Duration
}

type entry[V any] struct {
	value V
	until time.Time
}

func New[K comparable, V any](ttl time.Duration) *Cache[K, V] {
	return &Cache[K, V]{
		items: make(map[K]entry[V]),
		ttl:   ttl,
	}
}

func (c *Cache[K, V]) Get(key K) (V, bool) {
	c.mu.Lock()
	defer c.mu.Unlock()
	if e, ok := c.items[key]; ok && time.Now().Before(e.until) {
		return e.value, true
	}
	var zero V
	return zero, false
}

func (c *Cache[K, V]) Put(key K, value V) {
	c.mu.Lock()
	defer c.mu.Unlock()
	c.items[key] = entry[V]{value: value, until: time.Now().Add(c.ttl)}
}

func (c *Cache[K, V]) Len() int {
	return len(c.items)
}
