using System.Threading;
using System.Threading.Tasks;

namespace Skid.Monitor.Client.Sdk
{
    public interface ISkidMonitorExtension
    {
        string Name { get; }

        ValueTask OnSignalAsync(SkidSignalContext context, CancellationToken cancellationToken);
    }

    public sealed class SkidSignalContext
    {
        public SkidSignalContext(string schema, string type, string signalJson)
        {
            Schema = schema;
            Type = type;
            SignalJson = signalJson;
        }

        public string Schema { get; }

        public string Type { get; }

        public string SignalJson { get; }
    }
}
