using System.Reflection;
using System.Runtime.Loader;
using System.Text.Json;
using Skid.Monitor.Client.Sdk;

var extensions = ExtensionLoader.LoadFromEnvironment();
Console.Error.WriteLine($"skid-monitor extension host ready: {extensions.Count} extension(s)");

using var cancellation = new CancellationTokenSource();
Console.CancelKeyPress += (_, args) =>
{
    args.Cancel = true;
    cancellation.Cancel();
};

while (!cancellation.IsCancellationRequested)
{
    var line = await Console.In.ReadLineAsync(cancellation.Token);
    if (line is null)
    {
        break;
    }

    if (string.IsNullOrWhiteSpace(line))
    {
        continue;
    }

    try
    {
        using var document = JsonDocument.Parse(line);
        var root = document.RootElement;
        var context = new SkidSignalContext(
            root.GetProperty("schema").GetString() ?? string.Empty,
            root.GetProperty("type").GetString() ?? string.Empty,
            root.GetProperty("signal").Clone());

        foreach (var extension in extensions)
        {
            await extension.OnSignalAsync(context, cancellation.Token);
        }
    }
    catch (Exception ex) when (ex is JsonException or KeyNotFoundException or InvalidOperationException)
    {
        Console.Error.WriteLine($"skid-monitor extension event rejected: {ex.Message}");
    }
}

internal static class ExtensionLoader
{
    private const string ExtensionPathsEnv = "SKID_MONITOR_DOTNET_EXTENSIONS";

    public static IReadOnlyList<ISkidMonitorExtension> LoadFromEnvironment()
    {
        var paths = Environment.GetEnvironmentVariable(ExtensionPathsEnv);
        if (string.IsNullOrWhiteSpace(paths))
        {
            return Array.Empty<ISkidMonitorExtension>();
        }

        var extensions = new List<ISkidMonitorExtension>();
        foreach (var path in paths.Split(Path.PathSeparator, StringSplitOptions.RemoveEmptyEntries | StringSplitOptions.TrimEntries))
        {
            LoadExtensions(path, extensions);
        }
        return extensions;
    }

    private static void LoadExtensions(string assemblyPath, List<ISkidMonitorExtension> extensions)
    {
        var fullPath = Path.GetFullPath(assemblyPath);
        var assembly = AssemblyLoadContext.Default.LoadFromAssemblyPath(fullPath);
        foreach (var type in assembly.GetTypes())
        {
            if (type.IsAbstract || !typeof(ISkidMonitorExtension).IsAssignableFrom(type))
            {
                continue;
            }

            if (Activator.CreateInstance(type) is ISkidMonitorExtension extension)
            {
                extensions.Add(extension);
                Console.Error.WriteLine($"loaded skid-monitor extension: {extension.Name} ({type.FullName})");
            }
        }
    }
}
