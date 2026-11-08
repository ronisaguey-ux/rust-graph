using System;
using System.Collections.Generic;

namespace Example.Models
{
    public enum Status
    {
        Active = 1,
        Suspended = 2,
        Closed = 3
    }

    public record Item(long Id, string Name, Status Status)
    {
        public bool Active => Status == Status.Active;

        public Item Rename(string name)
        {
            if (string.IsNullOrWhiteSpace(name))
            {
                throw new ArgumentException(nameof(name));
            }
            return this with { Name = name };
        }
    }

    public interface IRepository
    {
        Item? Find(long id);
        void Save(Item item);
        IEnumerable<Item> All();
    }

    public sealed class Audit
    {
        public required string Action { get; init; }
        public DateTime At { get; init; } = DateTime.UtcNow;
        public IDictionary<string, string> Extra { get; } = new Dictionary<string, string>();
    }
}
