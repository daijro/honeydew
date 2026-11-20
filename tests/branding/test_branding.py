"""Static cross-platform brand contract. Run with python3 -m unittest discover -s tests/branding."""
from pathlib import Path
import re
import tomllib
import unittest
import xml.etree.ElementTree as ET

ROOT = Path(__file__).resolve().parents[2]

class BrandingTests(unittest.TestCase):
    def test_rust_package_names_and_dependency_paths(self):
        manifests = [ROOT/'Cargo.toml', *ROOT.glob('crates/*/Cargo.toml'), *ROOT.glob('vendor/banana-studio/*/Cargo.toml')]
        for manifest in manifests:
            data = tomllib.loads(manifest.read_text())
            self.assertNotIn('lemon', data.get('package', {}).get('name', ''))
            self.assertNotIn('honeydew', data.get('package', {}).get('name', ''))
            def walk(value):
                if isinstance(value, dict):
                    if 'path' in value:
                        self.assertTrue((manifest.parent / value['path']).exists(), (manifest, value['path']))
                    for child in value.values(): walk(child)
                elif isinstance(value, list):
                    for child in value: walk(child)
            walk(data)
        self.assertEqual(tomllib.loads((ROOT/'Cargo.toml').read_text())['package']['name'], 'banaforge')

    def test_android_package_java_jni_and_library_agree(self):
        gradle = (ROOT/'android/app/build.gradle').read_text()
        self.assertIn("applicationId = 'dev.daijro.banaforge'", gradle)
        self.assertIn("'app_name', 'BanaForge'", gradle)
        java = ROOT/'android/app/src/main/java/dev/daijro/banaforge'
        for path in java.glob('*.java'):
            self.assertIn('package dev.daijro.banaforge;', path.read_text())
            self.assertIn('class '+path.stem, path.read_text())
        self.assertIn('System.loadLibrary("banaforge")', (java/'BanaForgeActivity.java').read_text())
        manifest = ET.parse(ROOT/'android/app/src/main/AndroidManifest.xml')
        ns = '{http://schemas.android.com/apk/res/android}'
        activity = manifest.find('application/activity')
        self.assertEqual(activity.attrib[ns+'name'], 'dev.daijro.banaforge.BanaForgeActivity')
        self.assertEqual(activity.find('meta-data').attrib[ns+'value'], 'banaforge')
        for path in (ROOT/'src').glob('android*.rs'):
            text = path.read_text()
            for name in re.findall(r'Java_dev_daijro_\w+', text):
                self.assertTrue(name.startswith('Java_dev_daijro_banaforge_BanaForge'), name)

    def test_embedded_assets_and_xml_resolve(self):
        for directory in ('src', 'crates'):
            for path in (ROOT/directory).rglob('*.rs'):
                for target in re.findall(r'include_(?:bytes|str)!\(\s*"([^"]+)"', path.read_text()):
                    self.assertTrue((path.parent/target).exists(), (path, target))
        res = ROOT/'android/app/src/main/res'
        for path in res.rglob('*.xml'):
            ET.parse(path)
        for name in ('ic_launcher_foreground', 'ic_launcher_monochrome', 'ic_splash_logo'):
            bitmap = ET.parse(res/f'drawable/{name}.xml').getroot()
            source = bitmap.attrib['{http://schemas.android.com/apk/res/android}src'].split('/')[1]
            self.assertTrue((res/f'drawable-nodpi/{source}.png').exists())

    def test_visible_plugin_names_are_banana_and_ids_stay_compatible(self):
        for path in (ROOT/'vendor/banana-studio').rglob('*.rs'):
            text = path.read_text()
            for name in re.findall(r'const NAME: [^=]+ = "([^"]+)"', text):
                self.assertNotIn('Lemon', name, path)
            for class_id in re.findall(r'VST3_CLASS_ID: \[u8; 16\] = \*b"([^"]+)"', text):
                self.assertEqual(len(class_id.encode()), 16, path)
        self.assertIn('pub const APP_NAME: &str = "BanaForge";', (ROOT/'src/lib.rs').read_text())
        self.assertIn('org.openstudio.lemon-vocoder', (ROOT/'src/plugins/builtin.rs').read_text())

if __name__ == '__main__': unittest.main()
