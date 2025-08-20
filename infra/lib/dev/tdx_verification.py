#!/usr/bin/env python3
"""
TDX verification script for NXCC development.
Verifies:
  1) TDX guest environment
  2) TDREPORT ioctl on /dev/tdx_guest
  3) TDX quote via TSM configfs (required)
"""

import os
import fcntl
import ctypes
import sys
import time
import subprocess


# ---------- Structures and IOCTL helpers ----------

class TDXReport(ctypes.Structure):
    _fields_ = [
        ("reportdata", ctypes.c_ubyte * 64),
        ("tdreport", ctypes.c_ubyte * 1024),
    ]


def _calc_ioctl_get_report0():
    # _IOC encoding per Linux uapi
    IOC_NRBITS = 8
    IOC_TYPEBITS = 8
    IOC_SIZEBITS = 14
    IOC_DIRBITS = 2

    IOC_NRSHIFT = 0
    IOC_TYPESHIFT = IOC_NRSHIFT + IOC_NRBITS
    IOC_SIZESHIFT = IOC_TYPESHIFT + IOC_TYPEBITS
    IOC_DIRSHIFT = IOC_SIZESHIFT + IOC_SIZEBITS

    IOC_WRITE = 1
    IOC_READ = 2

    def _IOC(direction, type_char, number, size):
        return ((direction << IOC_DIRSHIFT)
                | (ord(type_char) << IOC_TYPESHIFT)
                | (number << IOC_NRSHIFT)
                | ((size & ((1 << IOC_SIZEBITS) - 1)) << IOC_SIZESHIFT))

    return _IOC(IOC_READ | IOC_WRITE, "T", 1, ctypes.sizeof(TDXReport))


# ---------- Checks ----------

def check_tdx_env():
    print("=== TDX Environment Verification ===")
    ok = True

    # CPU flags
    try:
        with open("/proc/cpuinfo", "r") as f:
            cpuinfo = f.read()
        if "tdx_guest" in cpuinfo:
            print("✅ TDX guest flag present")
        else:
            print("❌ TDX guest flag missing")
            ok = False
    except Exception as e:
        print(f"❌ CPU flag check failed: {e}")
        ok = False

    # Device
    dev = None
    for cand in ("/dev/tdx_guest", "/dev/tdx-guest"):
        if os.path.exists(cand):
            dev = cand
            break
    if dev:
        print(f"✅ TDX device: {dev}")
    else:
        print("❌ TDX device not found")
        ok = False

    # configfs mount
    if os.path.ismount("/sys/kernel/config"):
        print("✅ configfs mounted")
    else:
        print("❌ configfs not mounted at /sys/kernel/config")
        ok = False

    # TSM report interface root
    tsm_root = "/sys/kernel/config/tsm/report"
    if os.path.exists(tsm_root):
        print("✅ TSM configfs interface: /sys/kernel/config/tsm/report")
    else:
        print("❌ TSM configfs interface missing at /sys/kernel/config/tsm/report")
        ok = False

    # Kernel dmesg hint
    try:
        res = subprocess.run(["dmesg"], capture_output=True, text=True, timeout=5)
        if "tdx: Guest detected" in res.stdout:
            print("✅ Kernel detected TDX guest")
        else:
            print("⚠️  Could not confirm TDX guest in dmesg")
    except Exception:
        print("⚠️  dmesg unavailable")

    return ok, dev


# ---------- Functional tests ----------

def test_tdreport_ioctl(dev_path):
    print("\nTesting TDREPORT ioctl...")
    try:
        fd = os.open(dev_path, os.O_RDWR)
        try:
            req = TDXReport()
            seed = b"NXCC_TDX_TEST_DATA".ljust(64, b"\x00")
            for i, b in enumerate(seed):
                req.reportdata[i] = b
            cmd = _calc_ioctl_get_report0()
            fcntl.ioctl(fd, cmd, req)
            report = bytes(req.tdreport)
            if len(report) != 1024:
                print(f"❌ Unexpected report size: {len(report)}")
                return False
            print("✅ TDREPORT ioctl ok (1024 bytes)")
            return True
        finally:
            os.close(fd)
    except Exception as e:
        print(f"❌ TDREPORT ioctl failed: {e}")
        return False


def test_tsm_configfs_quote():
    print("\nTesting TSM configfs quote...")
    tsm_root = "/sys/kernel/config/tsm/report"
    if not os.path.exists(tsm_root):
        print("❌ TSM configfs not found")
        return False

    req_dir = os.path.join(tsm_root, f"req_{int(time.time())}")
    try:
        os.makedirs(req_dir)
    except Exception as e:
        print(f"❌ Cannot create request dir under configfs (need CAP_SYS_ADMIN?): {e}")
        return False

    try:
        # Optional: read auto-selected provider (RO attribute)
        provider_path = os.path.join(req_dir, "provider")
        if os.path.exists(provider_path):
            try:
                with open(provider_path, "r") as f:
                    print(f"Provider: {f.read().strip()}")
            except Exception:
                pass  # ignore read issues

        # Provide exactly 64 bytes REPORTDATA
        reportdata = b"NXCC_TSM_TEST_DATA".ljust(64, b"\x00")
        try:
            with open(os.path.join(req_dir, "inblob"), "wb") as f:
                f.write(reportdata)
        except Exception as e:
            print(f"❌ Writing inblob failed: {e}")
            return False

        # Read quote
        try:
            with open(os.path.join(req_dir, "outblob"), "rb") as f:
                out = f.read()
        except Exception as e:
            print(f"❌ Reading outblob failed: {e}")
            return False

        if not out:
            print("❌ outblob is empty")
            return False

        print(f"✅ TSM quote ok ({len(out)} bytes)")
        return True

    finally:
        # Best-effort cleanup
        try:
            os.rmdir(req_dir)
        except Exception:
            pass


# ---------- Main ----------

def main():
    print("=== NXCC TDX Verification Script ===\n")

    env_ok, dev = check_tdx_env()
    if not env_ok:
        print("\n❌ Environment checks failed")
        return 1

    td_ok = test_tdreport_ioctl(dev)
    tsm_ok = test_tsm_configfs_quote()

    print("\n=== Verification Summary ===")
    print(f"TDREPORT ioctl: {'OK' if td_ok else 'FAIL'}")
    print(f"TSM configfs quote: {'OK' if tsm_ok else 'FAIL'}")

    overall = td_ok and tsm_ok
    if overall:
        print("\n✅ TDX attestation ready (ioctl + TSM quote)")
        return 0
    else:
        print("\n❌ Attestation not ready")
        if not td_ok:
            print("   Fix TDREPORT ioctl: device, driver, or ABI mismatch.")
        if not tsm_ok:
            print("   Fix TSM configfs: mount configfs, ensure CAP_SYS_ADMIN, write 64-byte inblob, read outblob.")
        return 1


if __name__ == "__main__":
    sys.exit(main())
