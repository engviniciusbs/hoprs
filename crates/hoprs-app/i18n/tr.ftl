# hoprs arayüz metni, Türkçe. Anahtarlar ve değişkenler en.ftl ile aynıdır; yalnızca metin
# değişir. Biçim için en.ftl başlığına bakın.

app-title = hoprs
already-running = hoprs zaten çalışıyor
cannot-open-profiles = Kayıtlı siteler açılamadı

# Kayıtlı site sayısı: "1 site", "3 site".
site-count =
    { $count ->
        [one] { $count } site
       *[other] { $count } site
    }

# Ayarlar sayfası: arayüz dili ve seçenekleri. Dil adları kendi dillerinde yazılır, bu
# yüzden tüm kataloglarda aynıdır.
settings-language = Dil
language-system = Sistem
language-english = English
language-pt-br = Português (Brasil)
language-tr = Türkçe

## Top bar

brand = hoprs
no-sessions = Açık oturum yok
go-to-anything = Herhangi bir yere git
go-to-anything-key = Ctrl K
sidebar-options = Kenar çubuğu seçenekleri
menu-show-workspaces = Çalışma alanlarını göster
menu-show-sites = Tüm siteleri göster
menu-open = Aç
menu-copy-path = Yolu kopyala
menu-refresh = Yenile
menu-show-in-panes = Panellerde göster
menu-open-files = Dosyaları aç
menu-close-other-tabs = Diğer sekmeleri kapat
key-refresh = F5

## The Ctrl+K palette

palette-placeholder = Bir sekmeye, siteye veya çalışma alanına git ya da bir eylem çalıştır
palette-workspaces-placeholder = Çalışma alanı aç
palette-no-results = Eşleşme yok
palette-group-action = Eylem
palette-group-tab = Sekme
palette-group-site = Site
palette-group-workspace = Çalışma alanı
palette-kind-terminal = terminal
palette-kind-files = dosyalar
palette-toggle-queue = Kuyruğu aç/kapat
palette-toggle-log = Günlüğü aç/kapat
palette-close-tab = Sekmeyi kapat
palette-close-settings = Ayarları kapat
palette-open-workspace = Çalışma alanı aç…
palette-close-all = Tümünü kapat: { $name }
update-palette = hoprs'u { $version } sürümüne güncelle

# Güncelleme kartı (sol altta), alt çubuk satırı ve Ayarlar → Genel.
# $version bir sürümdür (ör. 1.4.0).
update-available = hoprs { $version } kullanıma hazır
# $version çalışan sürümdür.
update-you-have = Sürümünüz: { $version }.
update-action = Güncelle
update-whats-new = Yenilikler
update-dismiss = Kapat
# $done ve $total boyutlardır (ör. 12,4 MB).
update-downloading = İndiriliyor… { $done } / { $total }
update-downloading-size = İndiriliyor… { $done }
update-verifying = Denetleniyor…
update-try-again = Yeniden dene
update-ready-windows = Yüklemeye hazır. hoprs kapanacak.
update-transfers-stop =
    { $count ->
        [one] 1 aktarım duracak; hoprs bir sonraki açılışında devam edecek.
       *[other] { $count } aktarım duracak; hoprs bir sonraki açılışında devam edecek.
    }
update-close-and-install = Kapat ve yükle
update-later = Sonra
update-installing = Yükleyici açılıyor…
update-linux-opened = .deb dosyaları için varsayılan uygulamayla açıldı. Ya da şunu çalıştırın:
update-linux-run = Yüklemek için bu komutu çalıştırın:
update-open-failed = Yükleyici açılmadı.
update-installer-open = Yükleyici açık. Bitirmek için hoprs'u kapatın.
update-close-hoprs = hoprs'u kapat
update-copy = Kopyala
update-macos-steps = hoprs penceresini kapatın, sonra hoprs'u Uygulamalar'a sürükleyip Değiştir'i seçin. macOS engellerse Sistem Ayarları'nda Gizlilik ve Güvenlik bölümünden Yine de Aç'ı seçin.
update-footer-available = { $version } mevcut
update-footer-install = { $version } sürümünü yükle
settings-update-check = Güncellemeleri otomatik denetle
settings-update-check-hint = hoprs günde bir kez GitHub'dan en son sürümü sorar. Sizinle ilgili hiçbir şey gönderilmez.
settings-check-now = Şimdi denetle
settings-update-checking = Denetleniyor…
settings-up-to-date = hoprs güncel
palette-edit = Düzenleyicide düzenle
palette-compare-folders = Klasörleri karşılaştır
palette-upload = Seçimi yükle
palette-download = Seçimi indir
palette-copy = Kopyala
palette-paste = Yapıştır
palette-find = Geçmişte ara
palette-key-copy = Ctrl Shift C
palette-key-paste = Ctrl Shift V
palette-key-find = Ctrl Shift F

## Sidebar

filter-sites = Siteleri filtrele
workspaces = Çalışma alanları
all-sites = Tüm siteler
sidebar-all-hidden = Her iki liste de gizli. ⋯ menüsünden gösterin.
sidebar-collapse = Daralt
sidebar-expand = Genişlet
no-workspaces = Henüz çalışma alanı yok
no-sites = Henüz site yok
import-filezilla = FileZilla'dan içe aktar
settings = Ayarlar
key-settings = Ctrl ,
key-settings-macos = ⌘ ,
new-site = Yeni site
no-matches = Eşleşen site yok
menu-open-all = Tümünü aç
menu-close-all = Tümünü kapat
workspace-open-tip = Çalışma alanı açık
edit = Düzenle
site-missing = Site yok
cannot-load-sites = Kayıtlı siteler yüklenemiyor
retry = Yeniden dene
unreadable-entry = Bu kayıt okunamıyor (daha yeni bir hoprs yazmış olabilir mi?)
open-profile-folder = sites.toml klasörünü aç
cannot-open-folder = Klasör açılamıyor: { $reason }
workspace-site-count-partial =
    { $total ->
        [one] { $total } siteden { $present }
       *[other] { $total } siteden { $present }
    }
unreadable-name-fallback = { $file } dosyasında { $number }. kayıt

## Site form

site-form-new = Yeni site
site-form-edit = Siteyi düzenle
field-name = Ad
field-name-hint = Varsayılan olarak host
field-host = Host
field-port = Port
field-user = Kullanıcı
field-protocol = Protokol
field-password = Parola
password-optional = İsteğe bağlı
password-replace = Değiştirmek için yazın
password-saved = Sistem anahtarlığında kayıtlı
forget-password = Kayıtlı parolayı unut
ssh-options = SSH
use-agent = ssh-agent kullan
key-file = Anahtar dosyası
key-file-hint = Özel anahtarın yolu (isteğe bağlı)
open-as = Şu olarak aç
open-as-terminal = Terminal
open-as-files = Dosyalar
open-as-both = Terminal ve dosyalar
ftp-options = FTP
ftp-mode = Aktarım kipi
ftp-passive = Pasif
ftp-active = Etkin
nat-workaround = Sunucunun gönderdiği pasif adresi yoksay (NAT)
start-folders = Başlangıç klasörleri
remote-dir = Uzak klasör
local-dir = Yerel klasör
folder-hint = İsteğe bağlı
host-required = Host gerekli
port-invalid = Port, 1 ile 65535 arasında bir sayı olmalı
save = Kaydet
saving = Kaydediliyor…
cancel = İptal
delete = Sil
keep = Koru
delete-site-confirm = Bu site silinsin mi? Tüm çalışma alanlarından çıkar ve kayıtlı parolası silinir.

## Workspace form

workspace-form-new = Yeni çalışma alanı
workspace-form-edit = Çalışma alanını düzenle
workspace-sites = Siteler
workspace-no-sites = Henüz site yok. Önce bir site oluşturun.
name-required = Ad gerekli
delete-workspace-confirm = Bu çalışma alanı silinsin mi? Siteleri korunur.

## Center

no-session-title = Açık oturum yok
no-session-hint = Bağlanmak için kenar çubuğundan bir site seçin.
keyring-unavailable = Sistem anahtarlığı kullanılamadı, bu yüzden kayıtlı parola kullanılmadı.
edit-site = Siteyi düzenle

## Tabs

close-tab = Sekmeyi kapat
new-terminal-tab = Yeni terminal sekmesi
new-files-tab = Yeni dosyalar sekmesi
terminal-overflowed = Çıktı çok hızlıydı ve kabuk kapatıldı
terminal-reopen = Yeniden aç
terminal-restart = Kabuğu yeniden başlat
terminal-shell-closed = Kabuk kapandı
terminal-open-failed = Kabuk açılamıyor
terminal-retry = Yeniden dene
terminal-waiting-for-connection = Bağlantı bekleniyor
terminal-input-dropped = Bazı girdiler atıldı: kabuk yeterince hızlı almadı
terminal-reconnected = — yeniden bağlanıldı —
terminal-search-placeholder = Geçmişte ara
terminal-search-next = Sonraki
terminal-search-previous = Önceki
terminal-search-close = Kapat
terminal-search-no-match = Eşleşme yok
shell-exited-code = Kabuk sonlandı (kod { $code })
shell-exited = Kabuk sonlandı

## File panes
local = Yerel
col-name = Ad
col-size = Boyut
col-modified = Değiştirilme
col-permissions = İzinler
sort-ascending = ↑
sort-descending = ↓
parent-dir = ..
path-separator = /
home-dir = ~
no-size = -
empty-folder = Bu klasör boş
drop-upload =
    { $count ->
        [one] { $count } öğeyi { $folder } konumuna yükle
       *[other] { $count } öğeyi { $folder } konumuna yükle
    }
drop-download =
    { $count ->
        [one] { $count } öğeyi { $folder } konumuna indir
       *[other] { $count } öğeyi { $folder } konumuna indir
    }
drop-upload-files =
    { $count ->
        [one] { $count } dosyayı { $folder } konumuna yükle
       *[other] { $count } dosyayı { $folder } konumuna yükle
    }
drop-upload-files-unknown = Dosyaları { $folder } konumuna yükle
drag-more = +{ $more }
drag-count = { $count }
reconnect = Yeniden bağlan
size-unit-b = B
size-unit-kb = KB
size-unit-mb = MB
size-unit-gb = GB
size-unit-tb = TB
size-unit-pb = PB
size-unit-eb = EB
month-1 = Oca
month-2 = Şub
month-3 = Mar
month-4 = Nis
month-5 = May
month-6 = Haz
month-7 = Tem
month-8 = Ağu
month-9 = Eyl
month-10 = Eki
month-11 = Kas
month-12 = Ara

## Queue
transfers = Aktarımlar
queue-active = Etkin
queue-queued = Sırada
queue-failed = Başarısız
queue-done = Bitti
queue-hide = Gizle
queue-show = Göster
no-transfers = Aktarım yok
no-transfers-hint = Dosyaları seçip Yükle veya İndir'e basın, diğer panele sürükleyin ya da buraya bırakın.
queue-loading = Aktarımlar yükleniyor…
nothing-active = Çalışan yok
nothing-queued = Bekleyen yok
nothing-failed = Başarısızlık yok
nothing-done = Henüz biten yok
clear-finished = Biten ve başarısız olanları temizle

## Technical log panel
log = Log
log-error = Hata
log-warn = Uyarı
log-info = Info
log-debug = Debug
filter-log = Günlüğü filtrele
copy-shown = Gösterilenleri kopyala
clear = Temizle
clear-hint = Yalnızca bu paneli boşaltır; yeni satırlar gelmeye devam eder.
log-level-hint = En az bu kadar önemli satırları gösterir. hoprs RUST_LOG ile başlatılmadıkça (örneğin RUST_LOG=debug) yalnızca info ve üstü kaydedilir.
no-log-lines = Henüz bir şey kaydedilmedi
no-log-matches = Eşleşen satır yok
log-lines =
    { $count ->
        [one] { $count } satır
       *[other] { $count } satır
    }
waiting-for-worker = İşçi bekleniyor
starting = Başlıyor…
transferring = Aktarılıyor…
dismissed = Kapatıldı
paused = Duraklatıldı
cancelled = İptal edildi
transfer-done = Bitti
file-exists = Dosya var —
overwrite = Üzerine yaz
resume = Devam et
skip = Atla
pause = Duraklat
apply-to-all = Tümüne uygula (bu oturum)
saved-at = { $time } kaydedildi
site-paused = Duraklatıldı — bağlantı koptu
total-workers =
    { $workers ->
        [one] toplam, { $workers } işçi
       *[other] toplam, { $workers } işçi
    }
policy-ask = sor
policy-overwrite = üzerine yaz
policy-skip = atla
policy-overwrite-if-newer = daha yeniyse üzerine yaz
policy-resume = devam et
session-policy = Çakışmalar: { $name } (bu oturum)
policy-if-newer = daha yeniyse
session-policy-tag = { $name }: oturum
queue-unavailable = Aktarım kuyruğu kullanılamıyor: { $reason }. Bu oturumdaki aktarımlar yeniden başlatmadan sonra korunmaz.
skip-exists = var
skip-not-newer = daha yeni değil
skip-already-complete = zaten tamamlanmış
skip-by-user = kullanıcı atladı
skip-symlink = sembolik bağlantılar izlenmez
skipped = Atlandı: { $reason }

## Status bar
not-connected = Bağlı değil

## Status bar and connection states
connecting = Bağlanılıyor…
connected = Bağlı
waiting-for-you = Sizi bekliyor
closing = Sekmeler kaydediliyor ve kapatılıyor…
reconnecting-now = Yeniden bağlanılıyor…
saved-folder-gone = Kayıtlı klasör { $saved } yok; bunun yerine en yakın klasör gösteriliyor.
reconnecting = Yeniden bağlanılıyor ({ $attempt }/{ $of })
disconnected = Bağlantı kesildi — { $reason }
auth-none = kimlik doğrulama yok
auth-agent = ssh-agent ile { $key_type }
auth-key-file = { $key_type } anahtarı
auth-password = parola
auth-keyboard-interactive = keyboard-interactive
ftp-auth-ftp = FTP (şifresiz)
ftp-auth-ftps-explicit = FTPS açık
ftp-auth-ftps-implicit = FTPS örtük
workspace-status = { $name }: { $connected } / { $sites } site

## File actions
new-folder = Yeni klasör
rename = Yeniden adlandır
permissions = İzinler…
key-new-folder = F7
key-rename = F2
key-delete = Del
key-edit = F4
upload = Yükle
download = İndir
key-transfer = Ctrl ↵
new-folder-name = Yeni klasör
name-empty = Ad boş olamaz
name-invalid = Ad / içeremez ve . ya da .. olamaz
name-invalid-windows = Bu diskte ad \ içeremez
name-spaces = Ad boşlukla başlayamaz veya bitemez
mixed-permissions = Karışık izinler: ilk öğeninki gösteriliyor
name-taken = Bu adda bir öğe zaten burada
dismiss = Kapat
creating-folder = Klasör oluşturuluyor…
renaming = Yeniden adlandırılıyor…
changing-permissions = İzinler değiştiriliyor…
delete-folders-note = Klasörler içindekilerle birlikte silinir.
permissions-title = İzinler
perm-owner = Sahip
perm-group = Grup
perm-others = Diğerleri
perm-read = Okuma
perm-write = Yazma
perm-execute = Çalıştırma
perm-octal = Octal
octal-invalid = 0 ile 7 arasında üç veya dört rakam
apply = Uygula
delete-question-one = { $name } silinsin mi?
delete-question-many = { $count } öğe silinsin mi?
and-more = ve { $count } tane daha
deleting =
    { $done ->
        [0] Siliniyor…
        [1] Siliniyor… 1 öğe
       *[other] Siliniyor… { $done } öğe
    }
deleted-then =
    { $done ->
        [1] 1 öğe silindi, ardından: { $error }
       *[other] { $done } öğe silindi, ardından: { $error }
    }

## Security and credential dialogs
host-key-title = Bilinmeyen host anahtarı
host-key-intro = hoprs bu sunucuya daha önce bağlanmadı. Güvenmeden önce parmak izini sunucu yöneticisiyle doğrulayın.
trust-and-connect = Güven ve bağlan
cert-untrusted-title = Güvenilmeyen sertifika
cert-untrusted-intro = Sunucunun sertifikası doğrulanamıyor. Güvenmeden önce parmak izini sunucu yöneticisiyle doğrulayın.
cert-changed-title = Sertifika değişti
cert-changed-intro = Sunucunun sertifikası daha önce güvendiğiniz sertifika değil. Biri bağlantıyı araya girerek dinliyor olabilir.
trust-certificate = Bu sertifikaya güven
label-host = Host
label-account = Hesap
label-key-type = Anahtar türü
label-fingerprint = Parmak izi (SHA-256)
label-reason = Neden
label-trusted-before = Önceden güvenilen
label-presented-now = Şimdi sunulan
label-key-file = Anahtar
password-title = Parola
passphrase-title = Anahtar parolası
field-passphrase = Parola
kbd-title = Kimlik doğrulama
save-in-keyring = Sistem anahtarlığına kaydet
save-in-keyring-hint = Yalnızca bağlantı başarılı olunca kaydedilir
connect = Bağlan
unlock = Kilidi aç
continue = Devam
dialogs-waiting = { $count } tane daha bekliyor
other-key-types = Bu host farklı bir anahtar türüyle biliniyor ({ $types }). Biri onun kimliğine bürünüyor olabilir.
cert-reason-expired = Süresi dolmuş
cert-reason-not-yet-valid = Henüz geçerli değil
cert-reason-wrong-name = Farklı bir ad için verilmiş
cert-reason-unknown-issuer = Bilinmeyen otorite (kendinden imzalı veya güvenilmeyen veren)

## Changed host key, in the tab
host-key-changed-title = Host anahtarı değişti
host-key-changed-body = Sunucu, kendisi için kayıtlı olandan farklı bir anahtar sundu. Bu, sunucu yeniden kurulduğunda ya da biri bağlantıyı araya girerek dinlediğinde olur. Eski kayıt known_hosts içinde durdukça hoprs bağlanmaz.
host-key-changed-session-body = Sunucu, bu oturumun bağlanırken doğruladığından farklı bir anahtar sundu. Bu, sunucu yeniden kurulduğunda ya da biri bağlantıyı araya girerek dinlediğinde olur. Yeni bir oturum anahtarı known_hosts ile yeniden denetler.
remove-old-entry = Eski kaydı kaldır…
close-and-reconnect = Kapat ve yeniden bağlan
reading-known-hosts = known_hosts okunuyor…
removing-known-hosts = Kaldırılıyor…
remove = Kaldır
connect-again = Yeniden bağlan
no-known-hosts-entry = known_hosts içinde bu host için artık kayıt yok. Yeni anahtarı denetlemek için yeniden bağlanın.
known-hosts-line = known_hosts, satır { $line }
remove-known-hosts-question =
    { $lines ->
    [1] { $host } için 1 satır known_hosts dosyasından kaldırılsın mı? Sonraki bağlantıda yeni anahtara güvenmeniz istenir.
   *[other] { $host } için { $lines_text } satır known_hosts dosyasından kaldırılsın mı? Sonraki bağlantıda yeni anahtara güvenmeniz istenir.
    }
also-covers-one-host = { $line }. satır başka host'ları da kapsıyor (bir desen veya ad listesi): kaldırırsanız bu anahtar onlar için de unutulur.
also-covers-other-hosts = { $lines } numaralı satırlar başka host'ları da kapsıyor (desenler veya ad listeleri): kaldırırsanız bu anahtarlar onlar için de unutulur.

## Import from FileZilla
import-title = FileZilla'dan içe aktar
import-step-file = 1  Dosya
import-step-folders = 2  Klasörler
import-step-report = 3  Rapor
import-file-label = FileZilla'nın site yöneticisi dosyası
import-file-hint = Genellikle FileZilla'nın yapılandırma klasöründeki sitemanager.xml. Parolalar sistem anahtarlığına gider.
import-path-required = sitemanager.xml dosyasını seçin
choose-file = Seç…
next = İleri
back = Geri
import-folders-question = FileZilla klasörleri çalışma alanına dönüştürülsün mü?
import-folders-hint = Evet: her klasör için, yoluyla adlandırılmış ve klasörün sitelerini içeren bir çalışma alanı. Hayır: klasör yolu her site adının önüne eklenir ("Work/Alpha").
import-folders-yes = Evet, çalışma alanı oluştur
import-folders-no = Hayır, adlarda kalsın
import-running = İçe aktarılıyor…
import-imported = İçe aktarıldı
import-skipped = Atlandı
import-workspaces-created = Oluşturulan çalışma alanları
import-workspaces-not-created = Oluşturulmayan çalışma alanları
import-failures = Hatalar
import-none = Yok
import-failed-keep-source = Bazı kayıtlar kaydedilemedi: sitemanager.xml dosyasını saklayın ve düzelince yeniden içe aktarın.
import-passwords-keep-source = Anahtarlık bu sitelerin parolalarını reddetti: sitemanager.xml dosyasını saklayın, parolalar hâlâ onda.
delete-sitemanager = sitemanager.xml dosyasını sil
delete-sitemanager-confirm = FileZilla'nın sitemanager.xml dosyası silinsin mi? FileZilla'da bu siteler artık olmayacak.
deleting-sitemanager = Siliniyor…
sitemanager-deleted = sitemanager.xml silindi
done = Bitti
pick-sitemanager = FileZilla'nın sitemanager.xml dosyasını seçin
pick-filter-xml = XML dosyaları

## Settings
settings-general = Genel
settings-general-description = Dil, güncellemeler ve sürüm.
settings-transfers-description = Aynı anda kaç dosyanın taşınacağı, var olan dosyalara ne olacağı ve bağlantıların nasıl açık kalacağı.
settings-compare-description = Klasörlerin ne zaman karşılaştırılacağı ve dışarıda bırakılan adlar.
settings-editing-description = Uzak dosyaları açan düzenleyici.
settings-terminal-description = Terminal sekmelerinin görünümü ve ne kadarını hatırlayacakları.
settings-backup-description = Sitelerinizi, çalışma alanlarınızı ve parolalarınızı bir dosyaya kaydedin ya da geri yükleyin.
settings-transfers = Aktarımlar
settings-workers = Site başına işçi
settings-workers-hint = Site başına paralel aktarım sayısı, 1 ile 16 arası. Sonraki bağlantılar için geçerlidir.
settings-policy = Dosya zaten varsa
settings-policy-hint = Sonraki aktarımlar için geçerlidir. Sor, seçimi kuyrukta gösterir.
settings-keepalive = Keepalive
settings-keepalive-hint = Boştaki bir bağlantının ne sıklıkta denetlendiği; sunucular ve yönlendiriciler bağlantıyı açık tutsun diye. Sonraki bağlantılar için geçerlidir.
seconds = { $secs } sn
settings-unreadable = settings.toml okunamıyor, bu yüzden varsayılanlar geçerli. Değişiklikler hoprs kapanana kadar sürer ve dosya olduğu gibi kalsın diye kaydedilmez: ayarları kaydetmek için dosyayı düzeltin veya silin.
settings-save-failed = settings.toml kaydedilemiyor
settings-terminal = Terminal
settings-font-size = Yazı tipi boyutu
settings-scrollback = Geçmiş satırları
settings-scrollback-hint = Bundan sonra açılan terminaller için geçerlidir.
settings-theme = Tema
theme-hoprs-dark = hoprs koyu
settings-policy-ask = Sor
settings-policy-overwrite = Üzerine yaz
settings-policy-skip = Atla
settings-policy-overwrite-if-newer = Daha yeniyse üzerine yaz
settings-policy-resume = Devam et
decrease = −
increase = +

## Backup
backup = Backup
backup-hint = Tüm site, çalışma alanı ve kayıtlı parolaları içeren tek bir şifreli dosya. Parolayı saklayın: o olmadan yedek açılamaz.
export-backup = Yedeği dışa aktar…
restore-backup = Yedeği geri yükle…
backup-file = Dosya
backup-password = Parola
backup-password-confirm = Parola (tekrar)
backup-path-required = Bir dosya seçin
backup-password-required = Bir parola girin
backup-passwords-differ = Parolalar eşleşmiyor
export = Dışa aktar
export-overwrite-confirm = Bu dosya zaten var. Yeni yedekle üzerine yazılsın mı?
restore = Geri yükle
exporting = Yedek şifreleniyor… bu birkaç saniye sürer
restoring = Yedeğin şifresi çözülüyor… bu birkaç saniye sürer
restore-mode = Kip
restore-merge = Birleştir
restore-merge-hint = Yenileri ekler; sahip olduğunuz siteler ve çalışma alanları kalır.
restore-replace = Değiştir
restore-replace-hint = Her şey yedekteki gibi olur: yedekte olmayan siteler, çalışma alanları ve parolalar kaldırılır.
restore-replace-confirm = Tüm siteler, çalışma alanları ve kayıtlı parolalar yedektekilerle değiştirilsin mi?
replace = Değiştir
export-failed = Yedek yazılmadı
restore-failed = Hiçbir şey geri yüklenmedi
backup-secret-errors = Geri yüklenemeyen parolalar
backup-missing-secrets = Parolası olmadan kaydedildi (anahtarlıkta yok)
backup-skipped-unreadable = Dışa aktarılmadı (okunamayan kayıtlar)
backup-skipped-sites = Zaten burada olan siteler (korundu)
backup-skipped-workspaces = Zaten burada olan çalışma alanları (korundu)
export-ellipsis = Dışa aktar…
export-everything = Tümünü dışa aktar…
export-title-site = "{ $name }" dışa aktar (site)
export-title-workspace = "{ $name }" dışa aktar (çalışma alanı, { $sites })
export-title-all = Tümünü dışa aktar ({ $sites }, { $workspaces })
export-password-hint = Bu parolayı saklayın: o olmadan dosya açılamaz.
export-partial-note = Bu dosya sitelerinizin yalnızca bir kısmını içerecek. Birleştir ile geri yükleyin; hoprs 1.5.0 ve öncesi bu dosyayı açamaz.
pick-export = hoprs yedeğini kaydet
pick-restore = Bir hoprs yedeği seçin
export-done = { $sites }, { $workspaces } ve { $secrets } dışa aktarıldı.
drive-name-required = Dosya için bir ad girin
# The pages the browser shows after connecting to Google Drive.
drive-page-connected = hoprs bağlandı. Bu sekmeyi kapatabilirsiniz.
drive-page-failed = hoprs bağlanamadı. hoprs'a dönüp yeniden deneyin.
export-destination = Hedef
export-destination-file = Bu bilgisayarda bir dosya
export-destination-drive = Google Drive
drive-connect = Google Drive'a bağlan…
drive-account = Hesap
drive-connected = Bağlı
drive-disconnect = Bağlantıyı kes
drive-file-name = Ad
drive-waiting = Tarayıcı bekleniyor…
drive-copy-link = Bağlantıyı kopyala
drive-browser-hint = Tarayıcı açılmadıysa bağlantıyı kopyalayıp kendiniz açın.
drive-session-only = Sistem anahtarlığı kullanılamıyor: bu bağlantı hoprs kapanana kadar geçerli.
drive-uploading = Google Drive'a yükleniyor…
drive-uploaded = Google Drive'a yüklendi: { $path }
drive-open = Drive'da aç
drive-save-local = Bu bilgisayara kaydet…
drive-saved-local = { $path } konumuna kaydedildi
drive-settings-line = Google Drive: bağlı
workspace-count =
    { $count ->
    [one] { $count } çalışma alanı
   *[other] { $count } çalışma alanı
    }
password-count =
    { $count ->
    [one] { $count } parola
   *[other] { $count } parola
    }
restore-done = { $sites }, { $workspaces } ve { $secrets } geri yüklendi.

## Remote editing
edit-file = Düzenle
edit-no-folders = Klasörler düzenlenemez
edit-no-links = Bağlantılar düzenlenemez
edit-too-large = 50 MB'tan büyük dosyalar düzenlenemez
waiting-for-connection = Bağlantı bekleniyor…
remote-may-be-truncated = O zamana kadar sunucudaki kopya eksik olabilir.
changed-on-server = Siz açtıktan sonra sunucuda değişti
keep-remote = Uzaktakini koru
save-a-copy = Kopya kaydet
reload-in-editor = Artık sunucudaki sürüm: düzenleyicinizde yeniden yükleyin.
upload-and-close = Bekleyen değişiklikleri yükle ve kapat
close-keep-copies = Kapat ve yerel kopyaları sakla
upload-and-quit = Yükle ve çık
quit-keep-copies = Çık ve yerel kopyaları sakla
checking-edits = Uzak düzenlemeler denetleniyor…
stop-keep-copies = Durdur ve yerel kopyaları sakla
uploading-edits = Yükleniyor… (Yerel kopyaları saklamak için Durdur)
server-copy-of = Sunucudaki kopya:
may-be-incomplete = (eksik olabilir)
kept-saves-stopped = Oturum kapandı: düzenleyicide yapılacak yeni kayıtlar yüklenmeyecek. Değişiklikleriniz kurtarılan kopyada; düzenlemeye devam etmek için dosyayı yeniden açın.
edit-one-file = Düzenlemek için bir dosya seçin
uploaded = Yüklendi
editing = Düzenleniyor
opening = Açılıyor
close-workspace-edits-title = Bu çalışma alanındaki bazı uzak düzenlemeler yüklenmedi
keeping-copies = Yerel kopyalar kaydediliyor…
close-edits-title = Bazı uzak düzenlemeler yüklenmedi
quit-edits-title = Bazı uzak düzenlemeler yüklenmedi
quit-edits-hint = Siz seçene kadar pencere açık kalır. Hiçbir şey silinmez.
edit-unsent = yüklenmedi
edit-conflict = sunucuda değişti
editor-command = Düzenleyici
editor-command-hint-windows = system, uzak dosyaları sistemin varsayılan düzenleyicisinde açar. Ya da bir komut yazın; dosyanın yolu en sona eklenir. Örneğin: notepad++.exe, code.cmd --wait.
editor-command-hint-macos = system, uzak dosyaları sistemin varsayılan düzenleyicisinde açar. Ya da bir komut yazın; dosyanın yolu en sona eklenir. Örneğin: open -W -a "Visual Studio Code" (tam yol da çalışır).
editor-command-hint = system, uzak dosyaları sistemin varsayılan düzenleyicisinde açar. Ya da bir komut yazın; dosyanın yolu en sona eklenir. Örneğin: gedit, code --wait.
editor-system-default = Sistem varsayılanı
editor-command-empty = Bir komut yazın
settings-editing = Uzaktan düzenleme
settings-compare = Klasör karşılaştırma
settings-compare-auto = Klasörleri otomatik karşılaştır
settings-compare-ignore = Karşılaştırırken yoksay
settings-compare-ignore-hint = Adlar virgülle ayrılır. * herhangi bir karakter dizisiyle eşleşir, örneğin *.log. Boş alan hiçbir şeyi yoksaymaz.
upload-failed = Yükleme başarısız: { $reason }
your-version-at = Sürümünüz şuraya kaydedildi: { $path }
copies-go-to = Gönderilmemiş değişiklikler şuraya kopyalanır: { $dir }
not-uploaded =
    { $count ->
    [1] 1 dosya yüklenemedi
   *[other] { $count } dosya yüklenemedi
    }
kept-notice =
    { $count ->
    [1] 1 gönderilmemiş düzenleme kurtarılan kopya olarak şuraya kaydedildi:
   *[other] { $count } gönderilmemiş düzenleme kurtarılan kopyalar olarak şuraya kaydedildi:
    }
delete-site-edits =
    { $count ->
    [1] Bu sitenin 1 dosyası düzenleme için açık: gönderilmemiş değişiklikleri kurtarılan kopya olarak saklanır, asla silinmez.
   *[other] Bu sitenin { $count } dosyası düzenleme için açık: gönderilmemiş değişiklikleri kurtarılan kopyalar olarak saklanır, asla silinmez.
    }

## Errors — SSH and SFTP

error-auth-cancelled = Kullanıcı iptal etti
error-sftp-subsystem-refused = Sunucu SFTP alt sistemini reddetti
error-sftp-start-failed = SFTP başlatılamadı
error-sftp-connection-lost = SFTP bağlantısı koptu: { $path }
error-sftp-operation-failed = SFTP işlemi başarısız: { $path }
error-sftp-cannot-resume-size = { $offset } konumundan devam edilemiyor: { $path } yalnızca { $size } bayt
error-sftp-cannot-resume-missing = Olmayan bir dosyaya devam edilemiyor: { $path }
error-sftp-path-too-long = Yol çok uzun: { $path }
error-sftp-mtime-out-of-range = Değiştirilme zamanı SFTP için aralık dışında: { $path }
error-auth-failed = Kimlik doğrulama başarısız
error-ssh-kbd-wrong-answer-count = Yanlış sayıda keyboard-interactive yanıtı
error-ssh-kbd-too-many-rounds = Çok fazla keyboard-interactive turu
error-ssh-too-many-channels = Bu bağlantıda çok fazla kanal var (sınır { $limit })
error-known-hosts-changed = known_hosts değişti; kayıtları yeniden gözden geçirin
error-known-hosts-invalid-host = known_hosts için geçersiz host adı
error-ssh-protocol-error = SSH protokol hatası
error-host-key-changed = { $host } için host anahtarı değişti
error-host-key-revoked = { $host } için host anahtarı iptal edildi
error-host-key-certificate-unsupported = Host sertifikaları desteklenmiyor
error-ssh-shell-closed = Kabuk kapandı
error-ssh-pty-refused = Sunucu PTY/kabuk isteğini reddetti
error-ssh-closed-during-request = { $what } isteği sırasında bağlantı kapandı
error-ssh-shell-request-timed-out = Kabuk isteği zaman aşımına uğradı

## Errors — FTP and TLS

error-ftp-login-failed = Oturum açılamadı: { $context }
error-ftp-server-closed = Sunucu bağlantıyı kapattı: { $context }
error-ftp-cannot-open-data-connection = Veri bağlantısı açılamıyor: { $context }
error-ftp-transfer-aborted = Aktarım iptal edildi: { $context }
error-ftp-reply-no-such-file = Böyle bir dosya veya klasör yok: { $context }
error-ftp-reply-permission-denied = İzin reddedildi: { $context }
error-ftp-file-unavailable = Dosya kullanılamıyor: { $context }
error-ftp-server-failed-file = Sunucu dosyayı işleyemedi: { $context }
error-ftp-storage-full = Sunucu depolaması dolu veya kota aşıldı: { $context }
error-ftp-name-not-allowed = Dosya adına izin verilmiyor: { $context }
error-ftp-unexpected-reply = Beklenmeyen FTP yanıtı: { $context }
error-ftp-connection-lost = Bağlantı koptu: { $context }
error-tls-failed = TLS başarısız: { $context }
error-ftp-unexpected-reply-path = Beklenmeyen FTP yanıtı: { $path }
error-ftp-name-line-break = FTP, satır sonu içeren bir adı kullanamaz: { $name }
error-ftp-listing-format = Sunucunun liste biçimi okunamıyor
error-ftp-connection-lost-path = Bağlantı koptu: { $path }
error-ftp-offset-out-of-range = { $offset } konumu aralık dışında: { $path }
error-ftp-cannot-continue = { $path } dosyasına { $offset }. bayttan devam edilemiyor
error-ftp-cannot-continue-missing = Olmayan bir dosyaya devam edilemiyor: { $path }
error-ftp-mtime-out-of-range = Değiştirilme zamanı FTP için aralık dışında: { $context }
error-tls-certificate-not-trusted = Sertifikaya güvenilmiyor
error-ftp-ftps-refused = Sunucu FTPS'yi reddetti: { $target }
error-ftp-tls-config-missing = FTPS için bir TLS yapılandırması gerekir
error-ftp-control-command-timed-out = Denetim komutu zaman aşımına uğradı
error-ftp-command-timed-out = Komut zaman aşımına uğradı
error-ftp-no-data-timeout = Bağlantı koptu: { $secs } sn boyunca veri gelmedi
error-ftp-upload-refused = Yükleme sunucu tarafından reddedildi: { $context }
error-ftp-write-after-end = Yüklemenin bitiminden sonra yazma: { $context }
error-tls-trusted-cert-unusable = Güvenilen sertifika kullanılamıyor: { $path }
error-tls-verifier-build-failed = Sertifika doğrulayıcı oluşturulamıyor
error-tls-configure-failed = TLS yapılandırılamıyor

## Errors — network, files and local disk

error-fs-permission-denied = İzin reddedildi: { $path }
error-fs-no-such-file = Böyle bir dosya veya klasör yok: { $path }
error-fs-not-a-regular-file = Normal bir dosya değil: { $path }
error-fs-is-a-directory = Bir klasör: { $path }
error-fs-not-a-directory = Bir klasör değil: { $path }
error-net-connection-timed-out = Bağlantı zaman aşımına uğradı
error-net-connection-failed = Bağlantı başarısız
error-net-not-connected = Bağlı değil
error-net-disconnected-by-user = Kullanıcı bağlantıyı kesti
error-net-connection-lost = Bağlantı koptu
error-local-invalid-path = Geçersiz yerel yol
error-local-not-a-file = dosya değil: { $path }
error-local-offset-beyond-end = { $offset } konumu { $path } dosyasının sonunu aşıyor
error-local-cannot-resume-missing = olmayan dosyaya devam edilemiyor: { $path }
error-local-is-a-directory = bir klasör: { $path }
error-local-not-a-directory = klasör değil: { $path }
error-local-set-modified-failed = set_modified görevi başarısız: { $reason }


## Errors — vault, profiles and backups
error-backup-unsupported-parameters = Desteklenmeyen yedek parametreleri
error-backup-not-a-backup = Bir hoprs yedeği değil
error-backup-truncated = Yedek dosyası eksik
error-backup-unsupported-version = Desteklenmeyen yedek sürümü
error-backup-invalid-key-length = Geçersiz yedek anahtarı uzunluğu
error-backup-random-failed = Sistem rastgele sayı üreteci başarısız oldu
error-backup-cannot-encrypt = Yedek şifrelenemiyor
error-backup-wrong-password = Yanlış parola veya bozuk dosya
error-backup-too-large = Yedek dosyası çok büyük
error-backup-cannot-build = Yedek oluşturulamıyor
error-backup-invalid-content = Geçersiz yedek içeriği
error-backup-password-empty = Yedek parolası boş
error-backup-cannot-read-secret = Yedek için bir gizli bilgi okunamıyor
error-backup-partial-no-replace = Bu dosya bir yedeğin yalnızca bir kısmını içeriyor. Birleştir ile geri yükleyin.
error-vault-keyring-error = Sistem anahtarlığı hatası
error-vault-keyring-unavailable = Sistem anahtarlığı kullanılamıyor
error-vault-unknown-site = Bilinmeyen site
error-vault-unknown-workspace = Bilinmeyen çalışma alanı
error-vault-workspace-unknown-site = Çalışma alanında bilinmeyen site
error-vault-site-secrets-not-deleted = Site kaldırıldı, ancak gizli bilgileri anahtarlıktan silinemedi
error-profile-invalid-path = Geçersiz profil yolu
error-profile-symlink-loop = Çok fazla sembolik bağlantı düzeyi
error-profile-newer-version = Profil daha yeni bir hoprs tarafından yazılmış
error-profile-cannot-read = { $file } okunamıyor
error-profile-duplicate-id = { $file } içinde yinelenen kimlik
error-profile-cannot-save = { $file } kaydedilemiyor
error-profile-site-id-unreadable = Site kimliği okunamayan bir kayıt tarafından kullanılıyor
error-profile-workspace-id-unreadable = Çalışma alanı kimliği okunamayan bir kayıt tarafından kullanılıyor

## Errors — transfers and the queue
error-transfer-already-finished = Aktarım zaten bitti
error-transfer-not-paused = Aktarım duraklatılmış değil
error-transfer-duplicate-name-no-retry = Klasör listesindeki yinelenen ad yeniden denenemez
error-transfer-not-failed-or-cancelled = Aktarım başarısız veya iptal edilmiş değil
error-transfer-not-awaiting-decision = Aktarım bir karar beklemiyor
error-transfer-size-mismatch = Aktarımdan sonra boyut uyuşmuyor
error-transfer-items-failed = { $failed } öğe başarısız oldu
error-transfer-duplicate-name = Klasör listesinde yinelenen ad
error-transfer-worker-panicked = İşçi çöktü
error-transfer-destination-not-file = Hedef normal bir dosya değil
error-transfer-invalid-listing-name = Klasör listesinde geçersiz ad
error-transfer-unknown = Bilinmeyen aktarım
error-transfer-engine-stopped = Aktarım motoru durdu
error-queue-database-error = Kuyruk veritabanı hatası
error-queue-newer-version = Aktarım kuyruğu hoprs'un daha yeni bir sürümü tarafından kaydedilmiş

## Errors — import, settings, editing and sessions
error-import-cannot-read = sitemanager.xml okunamıyor
error-import-file-too-large = sitemanager.xml çok büyük
error-import-delete-symlink = silinmiyor: yol bir sembolik bağlantı
error-import-delete-not-regular = silinmiyor: normal bir dosya değil
error-import-delete-not-filezilla = silinmiyor: bir FileZilla site yöneticisi dosyası değil
error-settings-cannot-read = settings.toml okunamıyor
error-update-bad-response = Güncelleme sunucusu beklenmeyen bir yanıt verdi
error-update-checksum-missing = { $name }, sürümün SHA256SUMS dosyasında yok
error-update-checksum-mismatch = İndirilen dosya sağlama toplamıyla uyuşmuyor
error-update-check-failed = Güncellemeler denetlenemedi
error-update-download-failed = Güncelleme indirilemedi
error-update-dev-build = Geliştirme sürümlerinde güncelleme denetimi kapalı (HOPRS_UPDATE_URL ayarlayın)
error-update-download-busy = Bu güncelleme zaten indiriliyor
error-app-opener-task-failed = Açıcı görevi başarısız
error-app-opener-failed = { $opener } başarısız
error-app-vault-task-failed = Kasa görevi başarısız
error-app-background-task-failed = Arka plan görevi başarısız
error-app-no-config-dir = Yapılandırma klasörü yok
error-app-drop-needs-site = Dosya bırakmak için bağlı bir site açın
error-app-drop-pane-busy = Uzak klasör meşgul veya henüz listelenmedi; birazdan yeniden bırakın
error-app-drop-not-connected = Site bağlı değil; dosya bırakmak için yeniden bağlanın
error-app-drop-unsupported = Bu yol yüklenemiyor
error-app-site-endpoint-changed = Sitenin host veya port ayarı değişti: bu oturum kapatıldı. Yeni ayarları kullanmak için yeniden bağlanın.
error-session-closed = Oturum kapandı
error-session-shell-closed = Kabuk kapandı
error-session-terminal-needs-ssh = Terminaller için bir SSH sitesi gerekir
error-fs-already-exists = Zaten var
error-edit-unknown = Böyle bir düzenleme yok
error-edit-only-files = Yalnızca dosyalar düzenlenebilir
error-edit-remote-changed = Uzak dosya değişti
error-edit-local-gone = Yerel dosya yok
error-edit-copy-exists = Kopya zaten var
error-edit-cannot-watch = Düzenlenen dosyalar izlenemiyor
error-edit-no-parent = Üst klasör yok
error-edit-unmatched-quote = Düzenleyici komutunda eşleşmeyen tırnak var
error-edit-no-editor = Düzenleyici komutu ayarlanmamış
error-edit-cannot-start-editor = Düzenleyici başlatılamıyor
error-edit-path-refused = Bu uzak yol yerel olarak düzenlenemez
error-drive-connect-cancelled = Google Drive bağlantısı iptal edildi
error-drive-connect-timed-out = Tarayıcı 5 dakika içinde yanıt vermedi
error-drive-connect-denied = Google Drive erişimi verilmedi
error-drive-connect-failed = Google Drive'a bağlanılamadı
error-drive-not-connected = Google Drive'a bağlı değil
error-drive-upload-failed = Google Drive'a yüklenemedi
error-drive-not-configured = Google Drive bu derlemede kullanılamıyor

# Klasör karşılaştırma paneli (paneller ile kuyruk arasında).
compare-button = Karşılaştır
compare-title = Farklar
compare-auto = Otomatik karşılaştır
compare-close = Farklar panelini kapat
compare-collapse = Daralt
compare-expand = Genişlet
compare-group-only-local = Yalnızca bu bilgisayarda ({ $count })
compare-group-only-remote = Yalnızca sunucuda ({ $count })
compare-group-different = Farklı ({ $count })
compare-group-unreadable = Okunamadı ({ $count })
compare-summary-only-here = { $count } yalnızca burada
compare-summary-only-server = { $count } yalnızca sunucuda
compare-summary-different =
    { $count ->
    [1] 1 farklı
   *[other] { $count } farklı
    }
compare-local-newer = Yerel daha yeni
compare-remote-newer = Sunucudaki daha yeni
compare-size-differs = Boyut farklı
compare-kind-differs = Dosya ve klasör
compare-unreadable = Okunamadı: { $error }
compare-queued = Sırada
compare-tip-local = Bu bilgisayar: { $when }
compare-tip-remote = Sunucu: { $when }
compare-upload-selected = Seçileni yükle
compare-download-selected = Seçileni indir
compare-identical =
    { $count ->
    [1] 1 aynı
   *[other] { $count } aynı
    }
compare-all-identical =
    { $count ->
    [1] İki klasör aynı (1 öğe)
   *[other] İki klasör aynı ({ $count } öğe)
    }
compare-running =
    { $count ->
    [1] Karşılaştırılıyor… 1 öğe
   *[other] Karşılaştırılıyor… { $count } öğe
    }
compare-paused = Şimdiye kadar { $count } öğe görüldü; liste kısmi.
compare-continue = Her şeyi karşılaştır
compare-stop = Durdur
compare-stopped = Karşılaştırma durduruldu: { $error }
compare-partial = Durduruldu; liste kısmi.
compare-again = Yeniden karşılaştır
compare-roots = { $local } ↔ { $remote }
compare-ellipsis = …
