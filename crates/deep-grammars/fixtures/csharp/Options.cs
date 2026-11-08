using System;
using System.Collections.Generic;

namespace Example.Configuration
{
    public class ServerOptions
    {
        public const int DefaultPort = 8080;

        public string Host { get; set; } = "localhost";
        public int Port { get; set; } = DefaultPort;
        public bool UseTls { get; set; }
        public TimeSpan Timeout { get; set; } = TimeSpan.FromSeconds(30);
        public IList<string> AllowedOrigins { get; set; } = new List<string>();

        public string BaseUrl => $"{(UseTls ? "https" : "http")}://{Host}:{Port}";

        public void Validate()
        {
            if (Port <= 0 || Port > 65535)
            {
                throw new ArgumentOutOfRangeException(nameof(Port), Port, "port out of range");
            }
            if (string.IsNullOrEmpty(Host))
            {
                Host = "localhost";
            }
        }

        public override string ToString()
        {
            return BaseUrl;
        }
    }
}
