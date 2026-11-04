# Additional LCD fonts

Eight additional font families are available: Google Sans, Google Sans Flex, Inter, Rajdhani, Orbitron, JetBrains Mono, Roboto Condensed and Oswald.

The original four font IDs and saved names are retained to preserve existing designs. During the Windows installer build, `tools/fetch-fonts.ps1` downloads font files and individual SIL OFL licence files from the official `google/fonts` repository. The full installer includes them, and the LCD renderer loads them from the application's `fonts/` folder. The installer does not install fonts system-wide or download them on the end user's computer.

**Testing:** The Windows build and installer have been successfully generated, and the application has been launched on a test system. Visual rendering of every font on a physical LCD and compatibility with all devices have not yet been comprehensively verified.

**Limitations:** The standalone EXE artifact does **not** bundle these font files; use the full installer for testing. Google Sans Flex variable axes other than weight are not exposed as individual controls. The build intentionally fails if the source font downloads are unavailable or the `google/fonts` paths change, rather than silently omitting fonts.
