using System.Globalization;
using System.Text.Json;
using LibreHardwareMonitor.Hardware;

// Bundled sensor helper used by the AIO Cooler Screen Windows service.
// Output: one JSON object per line, with the library's own hardware/sensor identifiers.
// Identifiers are NEVER mapped to a hard-coded fan/pump number.

int intervalMs = 1000;
bool once = args.Contains("--once", StringComparer.OrdinalIgnoreCase);
for (int i = 0; i < args.Length; i++)
{
    if (args[i] == "--interval-ms" && i + 1 < args.Length &&
        int.TryParse(args[i + 1], CultureInfo.InvariantCulture, out int requested))
        intervalMs = Math.Clamp(requested, 250, 60000);
}

using var cts = new CancellationTokenSource();
Console.CancelKeyPress += (_, e) => { e.Cancel = true; cts.Cancel(); };

try
{
    var computer = new Computer
    {
        IsCpuEnabled = true,
        IsGpuEnabled = true,
        IsMotherboardEnabled = true,
        IsControllerEnabled = true,
        IsMemoryEnabled = true,
        IsStorageEnabled = true,
        IsNetworkEnabled = true,
        IsPowerMonitorEnabled = true,
    };
    computer.Open();
    do
    {
        var readings = new List<SensorReading>();
        foreach (IHardware hardware in computer.Hardware)
            Collect(hardware, readings);

        var snapshot = new SensorSnapshot(
            SchemaVersion: 1,
            CapturedAtUtc: DateTimeOffset.UtcNow,
            Readings: readings);
        Console.WriteLine(JsonSerializer.Serialize(snapshot));
        Console.Out.Flush();
        if (once) break;
        try { await Task.Delay(intervalMs, cts.Token); }
        catch (OperationCanceledException) { break; }
    } while (!cts.IsCancellationRequested);
    computer.Close();
}
catch (Exception ex)
{
    Console.Error.WriteLine($"Sensor discovery failed: {ex.GetType().Name}: {ex.Message}");
    Environment.ExitCode = 1;
}

static void Collect(IHardware hardware, List<SensorReading> readings)
{
    try { hardware.Update(); }
    catch (Exception ex)
    {
        Console.Error.WriteLine($"Cannot update hardware '{hardware.Name}': {ex.Message}");
        return;
    }
    foreach (ISensor sensor in hardware.Sensors)
    {
        if (sensor.Value is not float value || float.IsNaN(value) || float.IsInfinity(value))
            continue;
        readings.Add(new SensorReading(
            Id: sensor.Identifier.ToString(),
            HardwareId: hardware.Identifier.ToString(),
            HardwareName: hardware.Name,
            HardwareType: hardware.HardwareType.ToString(),
            Name: sensor.Name,
            Type: sensor.SensorType.ToString(),
            Value: value));
    }
    foreach (IHardware subHardware in hardware.SubHardware)
        Collect(subHardware, readings);
}

record SensorReading(string Id, string HardwareId, string HardwareName,
    string HardwareType, string Name, string Type, float Value);
record SensorSnapshot(int SchemaVersion, DateTimeOffset CapturedAtUtc,
    List<SensorReading> Readings);
