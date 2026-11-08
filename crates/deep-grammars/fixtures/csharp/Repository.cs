using System;
using System.Collections.Generic;
using System.Linq;
using Example.Models;

namespace Example.Storage
{
    public class MemoryRepository : IRepository
    {
        private readonly IDictionary<long, Item> rows = new Dictionary<long, Item>();
        private long nextId = 1;

        public Item? Find(long id)
        {
            return rows.TryGetValue(id, out var item) ? item : null;
        }

        public void Save(Item item)
        {
            if (item.Id == 0)
            {
                item = item with { Id = nextId++ };
            }
            rows[item.Id] = item;
        }

        public IEnumerable<Item> All()
        {
            return rows.Values;
        }

        public int DeleteClosed()
        {
            var closed = rows.Values.Where(item => item.Status == Status.Closed).ToList();
            foreach (var item in closed)
            {
                rows.Remove(item.Id);
            }
            return closed.Count;
        }
    }
}
