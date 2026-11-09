using System.Diagnostics;
using System.Text.Json;

if (args.Length == 0)
    throw new ArgumentException("usage: AdapterInspector EXECUTABLE [ARGS...]");
var start = new ProcessStartInfo(args[0]) { RedirectStandardInput = true, RedirectStandardOutput = true, UseShellExecute = false };
foreach (var arg in args.Skip(1)) start.ArgumentList.Add(arg);
using var child = Process.Start(start) ?? throw new InvalidOperationException("Cannot start adapter");
var commands = new[] { "hello", "list_scenarios", "shutdown" };
for (var i = 0; i < commands.Length; i++)
{
    var data = commands[i] == "hello" ? new { versions = new[] { 1 } } : (object)new { };
    await child.StandardInput.WriteLineAsync(JsonSerializer.Serialize(new { version = 1, id = i + 1, command = commands[i], data }));
    var line = await child.StandardOutput.ReadLineAsync() ?? throw new InvalidOperationException("Adapter disconnected");
    using var reply = JsonDocument.Parse(line);
    if (reply.RootElement.GetProperty("id").GetDouble() != i + 1 || reply.RootElement.GetProperty("version").GetDouble() != 1 || !reply.RootElement.GetProperty("ok").GetBoolean())
        throw new InvalidOperationException($"Invalid {commands[i]} response: {line}");
    Console.WriteLine($"{commands[i]}: {line}");
}
await child.WaitForExitAsync();
if (child.ExitCode != 0) throw new InvalidOperationException($"Adapter exited with {child.ExitCode}");
