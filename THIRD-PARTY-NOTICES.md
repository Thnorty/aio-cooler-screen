# Third-party notices

AIO Cooler Screen is MIT-licensed (see LICENSE). The installer also installs or uses the
following separate components, each under its own licence.

## PawnIO driver

- Installed by `PawnIO_setup.exe` (official signed build, version 2.2.0) when it isn't already
  present. It isn't removed when AIO Cooler Screen is uninstalled, because other tools may use it.
  Remove it from *Apps & Features* if you no longer need it.
- Copyright (C) namazso. Licensed under GPL-2.0 with an exception for independent programs that
  communicate with it only through its device I/O control interface, which is how AIO Cooler
  Screen uses it.
- Source: https://github.com/namazso/PawnIO and https://github.com/namazso/PawnIO.Setup.
  Downloads: https://pawnio.eu

## PawnIO modules (AMDFamily17.bin, IntelMSR.bin)

- From PawnIO.Modules release 0.2.11; copied to `pawnio\` in the install folder.
- Copyright (C) namazso. Licensed under LGPL-2.1; the full licence text is installed as
  `pawnio\COPYING-modules.txt`.
- Source: https://github.com/namazso/PawnIO.Modules

## ffmpeg (optional)

- Downloaded only when you choose to, from the gyan.dev "release essentials" build, into
  `ffmpeg\` in the install folder. Its licence (GPL) is saved there as `LICENSE.txt`.
- Source: https://ffmpeg.org, builds: https://www.gyan.dev/ffmpeg/builds/

## Slint

- The user interface uses Slint under the Slint Royalty-free Desktop, Mobile, and Web
  Applications License; see the credit on the About page. https://slint.dev

## LibreHardwareMonitorLib (.NET internal sensor helper)

- Bundled library: LibreHardwareMonitorLib 0.9.6. Licensed under **MPL-2.0**.
- Project and source: https://github.com/LibreHardwareMonitor/LibreHardwareMonitor
- Package: https://www.nuget.org/packages/LibreHardwareMonitorLib/0.9.6
- The MPL-2.0 license text is included at `licenses/MPL-2.0.txt` in this repository and the Windows installation. The upstream source is linked above.
- When distributing binaries, make the MPL-covered component's corresponding source available according to MPL-2.0 and review the full transitive dependency notices; this summary alone is not a complete dependency-license audit.

## .NET runtime

- The sensor helper is published with .NET 8 runtime components. Review their licenses and runtime redistribution obligations before distributing the installer.
