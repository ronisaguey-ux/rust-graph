using System;
using System.Threading.Tasks;
using Example.Configuration;
using Example.Web;

namespace Example
{
    internal static class Program
    {
        private static async Task<int> Main(string[] args)
        {
            var options = new ServerOptions();
            if (args.Length > 0)
            {
                options.Port = ParsePort(args[0], options.Port);
            }
            options.Validate();
            Console.WriteLine($"listening on {options.BaseUrl}");

            try
            {
                await StartAsync(options);
            }
            catch (Exception error)
            {
                Console.Error.WriteLine(error.Message);
                return 1;
            }
            return 0;
        }

        private static int ParsePort(string text, int fallback)
        {
            return int.TryParse(text, out int port) ? port : fallback;
        }

        private static Task StartAsync(ServerOptions options)
        {
            var tasks = new List<Task>();
            for (int i = 0; i < 3; i++)
            {
                tasks.Add(Task.Run(() => Console.WriteLine($"worker {i}")));
            }
            return Task.WhenAll(tasks);
        }
    }
}
