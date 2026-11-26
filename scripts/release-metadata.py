#!/usr/bin/env python3
"""Create the catalog from actual native packages, never partial release assets."""
import hashlib
import json
from pathlib import Path
import re
import sys
import os
import subprocess
import zipfile
import plistlib
import xml.etree.ElementTree as ET

version, directory = sys.argv[1:]
assert re.fullmatch(r"0|[1-9]\d*", version.split('.')[0]) and re.fullmatch(r"\d+\.\d+\.\d+", version), 'invalid version'
root = Path(directory)
base = f'https://github.com/MohammedSafwan10/ottero-releases/releases/download/v{version}'
mac = root / f'Ottero-{version}-macos-arm64.zip'
windows = list(root.glob(f'Calmfold-{version}-full.nupkg'))
assert len(windows) == 1, 'exact Windows full package required'
expected_license=Path('LICENSE').read_text().replace('\r\n','\n')
revision=os.environ.get('GITHUB_SHA') or subprocess.check_output(['git','rev-parse','HEAD'],text=True).strip()
assert re.fullmatch(r'[0-9a-f]{40}',revision)
def verify_notices(archive, prefix):
    assert archive.read(prefix+'LICENSE').decode().replace('\r\n','\n') == expected_license
    assert archive.read(prefix+'NOTICE').decode().startswith('Ottero by Nexdark Labs')
    assert len(archive.read(prefix+'THIRD_PARTY_NOTICES.txt')) > 1000
    assert revision in archive.read(prefix+'SOURCE.txt').decode()
with zipfile.ZipFile(mac) as archive:
    verify_notices(archive,'Ottero.app/Contents/Resources/')
    info = plistlib.loads(archive.read('Ottero.app/Contents/Info.plist'))
    assert info['CFBundleVersion'] == version and info['CFBundleShortVersionString'] == version
with zipfile.ZipFile(windows[0]) as archive:
    licenses=[name for name in archive.namelist() if name.endswith('/LICENSE')]
    assert len(licenses)==1, 'Windows package license missing or ambiguous'
    verify_notices(archive,licenses[0].removesuffix('LICENSE'))
    nuspecs = [n for n in archive.namelist() if n.endswith('.nuspec')]
    assert len(nuspecs) == 1
    metadata = ET.fromstring(archive.read(nuspecs[0]))
    assert any(node.tag.rsplit('}',1)[-1] == 'version' and node.text == version for node in metadata.iter())
portable=list(root.glob('*Portable.zip'))
assert len(portable)==1
with zipfile.ZipFile(portable[0]) as archive: verify_notices(archive,'current/')
entries = json.loads(Path('apps/desktop/changelog.json').read_text())
notes = next(e for e in entries if e['version'] == version)
platforms = {}
for platform, artifact in [('macos-arm64', mac), ('windows-x64', windows[0])]:
    with artifact.open('rb') as file: digest = hashlib.file_digest(file, 'sha256').hexdigest()
    size = artifact.stat().st_size
    assert 0 < size <= 300 * 1024 * 1024, 'package exceeds client size limit'
    platforms[platform] = dict(url=f'{base}/{artifact.name}', sha256=digest, size=size)
assert (root/'appcast.xml').exists() and list(root.glob('*Setup.exe')) and list(root.glob('*Portable.zip'))
(root/'stable.json').write_text(json.dumps(dict(version=version,source={'commit':revision,'url':f'https://github.com/MohammedSafwan10/ottero/tree/{revision}'},date=notes['date'],notes=[item for section in notes['sections'] for item in section['items']],platforms=platforms), indent=2)+'\n')
print('Verified package versions and generated complete release catalog')
