#!/usr/bin/env python3
"""Sign native Mac metadata with explicit file credentials; never use Keychain."""
import base64
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import xml.etree.ElementTree as ET

version, directory, key = sys.argv[1:]
root = Path(directory)
signer = Path(os.environ.get('OTTERO_SPARKLE_DIR','target/tools/sparkle')).resolve() / 'bin/sign_update'
public = base64.b64decode(Path('native/macos/sparkle-public-key.txt').read_text(), validate=False)
with tempfile.TemporaryDirectory() as tmp:
    secret = base64.b64decode(Path(key).read_text().strip(), validate=True)
    assert len(secret) == 32, 'invalid Sparkle seed'
    der = Path(tmp)/'seed.der'; der.write_bytes(bytes.fromhex('302e020100300506032b657004220420')+secret); der.chmod(0o600)
    derived = subprocess.check_output(['openssl','pkey','-inform','DER','-in',str(der),'-pubout','-outform','DER'])[-32:]
    assert derived == public, 'Sparkle private/public key mismatch'
archive = root/f'Ottero-{version}-macos-arm64.zip'
signature = subprocess.check_output([str(signer),'--ed-key-file',key,'-p',str(archive)],text=True).strip()
subprocess.run([str(signer),'--ed-key-file',key,'--verify',str(archive),signature],check=True)
ns = 'http://www.andymatuschak.org/xml-namespaces/sparkle'
ET.register_namespace('sparkle',ns)
rss=ET.Element('rss',version='2.0'); channel=ET.SubElement(rss,'channel')
ET.SubElement(channel,'title').text='Ottero'
item=ET.SubElement(channel,'item'); ET.SubElement(item,'title').text=f'Ottero {version}'
ET.SubElement(item,f'{{{ns}}}minimumSystemVersion').text='12.0'
ET.SubElement(item,'enclosure',{'url':f'https://github.com/MohammedSafwan10/ottero-releases/releases/download/v{version}/{archive.name}',f'{{{ns}}}version':version,f'{{{ns}}}shortVersionString':version,f'{{{ns}}}edSignature':signature,'length':str(archive.stat().st_size),'type':'application/octet-stream'})
appcast=root/'appcast.xml'; ET.ElementTree(rss).write(appcast,encoding='utf-8',xml_declaration=True)
subprocess.run([str(signer),'--ed-key-file',key,str(appcast)],check=True)
subprocess.run([str(signer),'--ed-key-file',key,'--verify',str(appcast)],check=True)
print('Verified archive and signed appcast against pinned Sparkle public key')
