using System;
using System.Threading;
using System.Threading.Tasks;
using Skid.Monitor.Client.Sdk;

namespace Skid.Monitor.Client.SampleExtension
{
    public sealed class SampleExtension : ISkidMonitorExtension
    {
        public string Name => "sample-signal-counter";

        private ulong _count;

        public ValueTask OnSignalAsync(SkidSignalContext context, CancellationToken cancellationToken)
        {
            _count++;
            Console.Error.WriteLine($"[{Name}] received {_count} {context.Type} event(s)");
            return new ValueTask();
        }
    }
}
