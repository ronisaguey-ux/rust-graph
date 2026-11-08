package com.example.cache;

import java.util.HashMap;
import java.util.Map;
import java.util.Optional;
import java.util.function.Function;

public final class Cache<K, V> {
    private final Map<K, Entry<V>> items = new HashMap<>();

    public synchronized V get(K key, Function<? super K, ? extends V> loader) {
        Entry<V> entry = items.get(key);
        if (entry == null || entry.expired()) {
            V value = loader.apply(key);
            items.put(key, new Entry<>(value, System.currentTimeMillis() + 60_000L));
            return value;
        }
        return entry.value;
    }

    public Optional<V> peek(K key) {
        Entry<V> entry = items.get(key);
        return entry == null ? Optional.empty() : Optional.of(entry.value);
    }

    private static final class Entry<V> {
        private final V value;
        private final long expiresAt;

        Entry(V value, long expiresAt) {
            this.value = value;
            this.expiresAt = expiresAt;
        }

        boolean expired() {
            return System.currentTimeMillis() > expiresAt;
        }
    }
}
