using System;
using System.Collections.Generic;

namespace Example.Caching
{
    public sealed class Cache<TKey, TValue> where TKey : notnull
    {
        private readonly Dictionary<TKey, Entry> items = new();
        private readonly TimeSpan ttl = TimeSpan.FromMinutes(5);

        public TValue Get(TKey key, Func<TKey, TValue> load)
        {
            if (!items.TryGetValue(key, out Entry entry) || entry.Expired())
            {
                TValue value = load(key);
                entry = new Entry(value, DateTime.UtcNow.Add(ttl));
                items[key] = entry;
            }
            return entry.Value;
        }

        public bool Remove(TKey key) => items.Remove(key);

        private sealed class Entry
        {
            public Entry(TValue value, DateTime expiresAt)
            {
                Value = value;
                ExpiresAt = expiresAt;
            }

            public TValue Value { get; }
            public DateTime ExpiresAt { get; }

            public bool Expired()
            {
                return DateTime.UtcNow > ExpiresAt;
            }
        }
    }
}
