package com.example.cache

import java.time.Duration
import java.time.Instant
import java.util.concurrent.ConcurrentHashMap

class Cache<K, V>(private val ttl: Duration = Duration.ofMinutes(5)) {
    private data class Entry<V>(val value: V, val expiresAt: Instant)

    private val items = ConcurrentHashMap<K, Entry<V>>()

    fun get(key: K, load: (K) -> V): V {
        val entry = items[key]
        if (entry == null || entry.expiresAt.isBefore(Instant.now())) {
            val value = load(key)
            items[key] = Entry(value, Instant.now().plus(ttl))
            return value
        }
        return entry.value
    }

    fun put(key: K, value: V) {
        items[key] = Entry(value, Instant.now().plus(ttl))
    }

    fun remove(key: K): Boolean = items.remove(key) != null

    val size: Int
        get() = items.size
}
