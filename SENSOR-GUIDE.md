# Hardware sensors / Donanım sensörleri

## English

**Internal LibreHardwareMonitor (LHM)** is bundled with the full installer. It reads supported hardware via a .NET 8 helper managed by the Windows service, without needing the separate LHM application or an HTTP server. Missing or inaccessible sensors are not guaranteed to appear. The service may require elevated hardware access. Sensor names and IDs depend on the hardware; there is **no universal pump/fan index**.

In **Sensors**, choose a specific source or Automatic. The design editor shows human-readable names, but saved designs retain stable source keys. If the selected sensor goes offline, the saved key remains for reconnection.

**Optional alternatives:** HWiNFO shared memory (enable *Shared Memory Support*) or external LibreHardwareMonitor (run as administrator, enable *Options → Remote Web Server → Run*, then configure its server port, default `8085`). The external HTTP port **does not control the bundled helper**.

**Smart App Control:** The helper is currently unsigned. Windows may refuse to load it, including with a .NET `FileLoadException` (`0x800711C7`). Do not turn off system security. Check *Event Viewer → Applications and Services Logs → Microsoft → Windows → CodeIntegrity → Operational* if it is blocked. Different build hashes are not a compatibility guarantee.

## Türkçe

**Dahili LibreHardwareMonitor (LHM)** tam kurulum paketine dahildir. Windows hizmetinin yönettiği .NET 8 yardımcı uygulaması üzerinden donanım sensörlerini okur; ayrıca LHM uygulaması veya HTTP sunucusu gerekmez. Donanımın sunmadığı ya da erişilemeyen sensörler gösterilmeyebilir. Donanım erişiminde yetki gerekebilir. **Tüm bilgisayarlara uyan sabit bir pompa/fan numarası yoktur.**

**Sensörler** bölümünden Otomatik veya belirli bir sensör seçebilirsiniz. Tasarımda anlaşılır adlar görünür, fakat kayıtlı tasarımlarda kararlı sensör anahtarları korunur.

**İsteğe bağlı alternatifler:** HWiNFO'da *Shared Memory Support*; harici LibreHardwareMonitor'de yönetici olarak *Options → Remote Web Server → Run* ve isteğe bağlı port (varsayılan `8085`). Bu port **dahili yardımcı uygulamayı etkilemez**.

**Smart App Control:** Yardımcı uygulama şu anda imzasızdır; Windows .NET `FileLoadException` (`0x800711C7`) ile çalışmasını engelleyebilir. Güvenlik özelliklerini kapatmayın. Engellemeyi Olay Görüntüleyicisi'nin *CodeIntegrity → Operational* günlüklerinden inceleyin. Yeni SHA-256 özeti tek başına uyumluluk garantisi değildir.
