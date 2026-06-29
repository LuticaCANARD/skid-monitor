using System.Text.Json;

namespace Skid.Monitor.Client.Sdk;

public interface ISkidMonitorExtension
{
    string Name { get; }

    ValueTask OnSignalAsync(SkidSignalContext context, CancellationToken cancellationToken);
}

public sealed class SkidSignalContext
{
    public SkidSignalContext(string schema, string type, JsonElement signal)
    {
        Schema = schema;
        Type = type;
        Signal = signal;
    }

    public string Schema { get; }

    public string Type { get; }

    public JsonElement Signal { get; }
}
