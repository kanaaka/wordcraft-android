"""Package the Java Activity into cargo-apk's native APK before final signing."""
import argparse
from pathlib import Path
import subprocess
import xml.etree.ElementTree as ET
import zipfile

parser = argparse.ArgumentParser()
for arg in ('workspace', 'port', 'stage', 'sdk'):
    parser.add_argument('--' + arg, type=Path, required=True)
args = parser.parse_args()
android = 'http://schemas.android.com/apk/res/android'
ET.register_namespace('android', android)
tree = ET.parse(args.workspace / 'target/release/apk/AndroidManifest.xml')
root = tree.getroot()
app = root.find('application')
assert app is not None
app.set(f'{{{android}}}hasCode', 'true')
# API 29 needs the opt-out even with READ/WRITE_EXTERNAL_STORAGE. Android 11+
# ignores it and uses the explicit all-files special-access grant instead.
app.set(f'{{{android}}}requestLegacyExternalStorage', 'true')
# Android's resolver matches MIME strings case-sensitively. Some file managers
# normalize macroEnabled MIME names; accept both spellings.
for intent_filter in app.findall('./activity/intent-filter'):
    for data in list(intent_filter.findall('data')):
        mime = data.get(f'{{{android}}}mimeType')
        if mime and mime != mime.lower():
            ET.SubElement(intent_filter, 'data', {f'{{{android}}}mimeType': mime.lower()})
root.set(f'{{{android}}}versionCode', '16778499')
root.set(f'{{{android}}}versionName', '0.5.0-files-ru.3')
manifest = args.stage / 'AndroidManifest.xml'
tree.write(manifest, encoding='utf-8', xml_declaration=True)
unsigned = args.stage / 'unsigned.apk'
subprocess.run([
    str(args.sdk / 'build-tools/35.0.0/aapt.exe'), 'package', '-f',
    '-M', str(manifest), '-S', str(args.port / 'apps/wordcraft-android/res'),
    '-I', str(args.sdk / 'platforms/android-35/android.jar'), '-F', str(unsigned),
], check=True)
with zipfile.ZipFile(args.workspace / 'target/release/apk/wordcraft.apk') as native:
    with zipfile.ZipFile(unsigned, 'a', compression=zipfile.ZIP_DEFLATED) as apk:
        for name in native.namelist():
            if name.startswith('lib/') and name.endswith('.so'):
                apk.writestr(name, native.read(name))
        apk.write(args.stage / 'dex/classes.dex', 'classes.dex')
with zipfile.ZipFile(unsigned) as apk:
    assert 'classes.dex' in apk.namelist()
    assert 'lib/arm64-v8a/libwordcraft_android.so' in apk.namelist()
