using System;
using System.Collections.Generic;
using System.Threading.Tasks;

namespace Example.Utilities
{
    public static class Extensions
    {
        public static string OrDefault(this string? value, string fallback)
        {
            return string.IsNullOrEmpty(value) ? fallback : value;
        }

        public static IEnumerable<T> DistinctBy<T, TKey>(this IEnumerable<T> source, Func<T, TKey> key)
        {
            var seen = new HashSet<TKey>();
            foreach (var item in source)
            {
                if (seen.Add(key(item)))
                {
                    yield return item;
                }
            }
        }

        public static async Task<T> WithTimeout<T>(this Task<T> task, int milliseconds)
        {
            var completed = await Task.WhenAny(task, Task.Delay(milliseconds));
            if (completed != task)
            {
                throw new TimeoutException($"timed out after {milliseconds}ms");
            }
            return await task;
        }

        public static int Count(this string text, char needle)
        {
            int total = 0;
            for (int i = 0; i < text.Length; i++)
            {
                if (text[i] == needle)
                {
                    total++;
                }
            }
            return total;
        }
    }
}
