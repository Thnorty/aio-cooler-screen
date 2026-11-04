# Build & install — AIO Cooler Screen v1.2.4-Fork

This fork is based on [Thnorty/aio-cooler-screen](https://github.com/Thnorty/aio-cooler-screen). Building requires Windows x64, Rust MSVC, .NET 8 SDK and Inno Setup 6.x with Turkish language support (and the source dependencies obtained by the build script). Preserve the MIT and third-party notices.

## GitHub Actions

`.github/workflows/build-windows.yml` is **manually triggered**, not automatically on every push. It builds three artifacts:

- `aio-cooler-screen-windows-setup` — full Windows installer, service, fonts and bundled sensor helper.
- `aio-cooler-screen-standalone-exe` — advanced standalone GUI/service binary, **not** full installation.
- `aio-cooler-screen-internal-sensor-helper` — sensor helper publish output for diagnostics.

To trigger a manual build in the **public fork**:

```powershell
gh workflow run build-windows.yml -R SefaMeert/aio-cooler-screen --ref main
gh run list -R SefaMeert/aio-cooler-screen --workflow build-windows.yml --limit 3
```

To build locally:

```powershell
dotnet publish sensor-probe/SensorProbe.csproj -c Release -r win-x64 --self-contained true -o sensor-probe-dist
./build-installer.ps1
```

The build script validates the downloaded PawnIO installer signature and checksums the downloaded modules. Optional FFmpeg is user-selected. The installer uses English/Turkish language options and tries to close only processes launched from its install folder, then stop the `AioScreen` service. If it cannot, installation should **abort rather than skip locked DLLs**.

**Unsigned builds:** Smart App Control / Code Integrity may block the helper even when compilation succeeds. Test with protections enabled. A different SHA-256 value alone does not make a file trusted.

Before publishing a Windows installer, test upgrades over a running v1.2.3 installation, clean installation and uninstall, service startup and restart, helper and sensor data, installer/tray translations, and both localized README screenshot sets. **Compilation and a successful setup build do not prove Smart App Control compatibility.** Keep release binaries clearly marked as unsigned until they are signed and verified.
