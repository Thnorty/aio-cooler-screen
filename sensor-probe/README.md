# Built-in hardware sensor helper (SensorProbe)

`SensorProbe` is the **integrated .NET 8 / LibreHardwareMonitorLib helper** used by AIO Cooler Screen v1.2.4-Fork. It is launched and monitored by the AIO Windows service; an external LibreHardwareMonitor GUI or HTTP server is **not** required. The existing external LibreHardwareMonitor and HWiNFO sources remain optional alternatives.

## Build

From the project root on Windows x64 with the .NET 8 SDK:

```powershell
dotnet publish sensor-probe/SensorProbe.csproj -c Release -r win-x64 --self-contained true -o sensor-probe-dist
```

The Windows installer build packages the published output alongside `aio-screen.exe`. Copying only the standalone `aio-screen.exe` is **not** equivalent to installing the full application. The single-file publish setting reduces the number of separately deployed assemblies but **does not guarantee acceptance by Smart App Control**.

## Runtime protocol

The helper emits one JSON object per line to standard output. Each snapshot contains `SchemaVersion`, `CapturedAtUtc`, and `Readings`; individual readings contain the library-provided sensor `Id`, hardware identifiers and names, sensor type, and numeric value. The Rust service reads these snapshots over a private pipe and uses the sensor's actual identifier in saved selection keys. No fan or pump index is hardcoded.

For one-shot diagnostics, run the helper from its published directory:

```powershell
.\SensorProbe.exe --once > sensors.json
```

Sensor names and hardware information may be sensitive; review diagnostic output before sharing. Reading some motherboard/CPU sensors requires suitable hardware-access privileges. Missing sensors should not be treated as a software crash or universally supported hardware.

## Troubleshooting and licensing

If Code Integrity blocks an unsigned executable or assembly, inspect the Windows **CodeIntegrity/Operational** log. Do not disable Windows protections simply to run a test build. Refer to [the sensor guide](../SENSOR-GUIDE.md) and [third-party notices](../THIRD-PARTY-NOTICES.md). LibreHardwareMonitorLib 0.9.6 is MPL-2.0.
