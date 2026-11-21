import struct
import unittest
from verify_android_native import ABIS, check_identity, check_dependencies, check_exports


class NativePackagingTests(unittest.TestCase):
    def test_real_elf_class_and_machine_must_match_directory(self):
        for abi, (bits, machine) in ABIS.items():
            elf = bytearray(64)
            elf[:6] = b'\x7fELF' + bytes([bits, 1])
            struct.pack_into('<H', elf, 18, machine)
            check_identity(elf, abi)
            other = next(a for a in ABIS if a != abi)
            with self.assertRaises(ValueError):
                check_identity(elf, other)
        with self.assertRaises(ValueError):
            check_identity(b'not an engine', 'armeabi-v7a')

    def test_cpp_runtime_cannot_be_satisfied_by_system_or_another_abi(self):
        with self.assertRaises(ValueError):
            check_dependencies(['libc.so', 'libc++_shared.so'], {'libbanaforge.so'})
        check_dependencies(['libc.so', 'libc++_shared.so'], {'libbanaforge.so', 'libc++_shared.so'})
        with self.assertRaises(ValueError):
            check_dependencies(['libmissing_plugin.so'], {'libbanaforge.so'})

    def test_activity_and_native_callbacks_must_be_exported(self):
        java = 'private static native void onMic(boolean granted);'
        symbols = {'ANativeActivity_onCreate', 'Java_dev_daijro_banaforge_BanaForgeActivity_onMic'}
        check_exports(symbols, java)
        for symbol in symbols:
            with self.assertRaises(ValueError):
                check_exports(symbols - {symbol}, java)


if __name__ == '__main__':
    unittest.main()
