using System;
using System.Collections.Generic;
using System.Threading;
using System.Threading.Tasks;

namespace Example.Jobs
{
    public sealed class WorkerPool
    {
        private readonly SemaphoreSlim gate;
        private int processed;

        public WorkerPool(int concurrency)
        {
            gate = new SemaphoreSlim(concurrency);
        }

        public async Task<int> RunAsync(IReadOnlyList<string> jobs, CancellationToken token)
        {
            var pending = new List<Task>();
            foreach (string job in jobs)
            {
                await gate.WaitAsync(token);
                pending.Add(Task.Run(() =>
                {
                    try
                    {
                        Execute(job);
                        Interlocked.Increment(ref processed);
                    }
                    finally
                    {
                        gate.Release();
                    }
                }, token));
            }
            await Task.WhenAll(pending);
            return processed;
        }

        private static void Execute(string job)
        {
            switch (job)
            {
                case "":
                    throw new ArgumentException("empty job");
                case "skip" when DateTime.Now.Second > 30:
                    break;
                default:
                    Console.WriteLine(job);
                    break;
            }
        }
    }
}
