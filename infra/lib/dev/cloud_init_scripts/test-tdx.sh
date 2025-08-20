#!/bin/bash
echo "=== NXCC TDX Hardware Verification ==="
echo ""

# Exit code tracker
EXIT_CODE=0

# Test 1: Check kernel TDX detection
echo "Test 1: Checking kernel TDX detection..."
if grep -q "tdx_guest" /proc/cpuinfo; then
	echo "✅ TDX guest environment detected in CPU flags"
else
	echo "❌ TDX guest not detected in CPU flags"
	EXIT_CODE=1
fi

# Test 2: Check TDX device availability
echo ""
echo "Test 2: Checking TDX device availability..."
TDX_DEVICE=""
if [ -c /dev/tdx_guest ]; then
	TDX_DEVICE="/dev/tdx_guest"
	echo "✅ TDX device available: $TDX_DEVICE"
	echo "   Permissions: $(ls -la $TDX_DEVICE)"
elif [ -c /dev/tdx-guest ]; then
	TDX_DEVICE="/dev/tdx-guest"
	echo "✅ TDX device available: $TDX_DEVICE"
	echo "   Permissions: $(ls -la $TDX_DEVICE)"
else
	echo "❌ No TDX device found (/dev/tdx_guest or /dev/tdx-guest)"
	EXIT_CODE=1
fi

# Test 3: Check TSM configfs support
echo ""
echo "Test 3: Checking TSM configfs support..."
if [ -d /sys/kernel/config/tsm/report ]; then
	echo "✅ TSM configfs interface available"
	echo "   Path: /sys/kernel/config/tsm/report"
else
	echo "❌ TSM configfs interface not found"
	echo "   This is needed for quote generation"
	EXIT_CODE=1
fi

# Test 4: Test actual TDX functionality
echo ""
echo "Test 4: Testing TDX TDREPORT generation..."
if [ -n "$TDX_DEVICE" ]; then
	python3 <<'PYEOF'
import os, fcntl, ctypes, sys

def test_tdreport():
    """Test TDREPORT generation using correct ioctl structure"""
    try:
        DEV = "/dev/tdx_guest" if os.path.exists("/dev/tdx_guest") else "/dev/tdx-guest"
        fd = os.open(DEV, os.O_RDWR)
        
        class Req(ctypes.Structure):
            _fields_ = [("reportdata", ctypes.c_ubyte*64),
                       ("tdreport",  ctypes.c_ubyte*1024)]
        
        # Calculate correct ioctl magic number  
        IOC_NRBITS=8; IOC_TYPEBITS=8; IOC_SIZEBITS=14; IOC_DIRBITS=2
        IOC_NRSHIFT=0; IOC_TYPESHIFT=IOC_NRSHIFT+IOC_NRBITS
        IOC_SIZESHIFT=IOC_TYPESHIFT+IOC_TYPEBITS
        IOC_DIRSHIFT=IOC_SIZESHIFT+IOC_SIZEBITS
        IOC_WRITE=1; IOC_READ=2
        
        def _IOC(d,t,n,s): 
            return (d<<IOC_DIRSHIFT)|(ord(t)<<IOC_TYPESHIFT)|(n<<IOC_NRSHIFT)|(((s)&((1<<IOC_SIZEBITS)-1))<<IOC_SIZESHIFT)
        
        GET_REPORT0 = _IOC(IOC_READ|IOC_WRITE, "T", 1, ctypes.sizeof(Req))
        
        req = Req()
        fcntl.ioctl(fd, GET_REPORT0, req)
        os.close(fd)
        
        print("✅ TDX TDREPORT generation successful")
        print(f"   Report size: {len(bytes(req.tdreport))} bytes")
        return True
    except Exception as e:
        print(f"❌ TDX TDREPORT failed: {e}")
        return False

# Run the test
if test_tdreport():
    sys.exit(0)
else:
    sys.exit(1)
PYEOF

	if test $? -eq 0; then
		echo "✅ TDREPORT generation verified"
	else
		echo "❌ TDREPORT generation failed"
		EXIT_CODE=1
	fi
else
	echo "❌ Cannot test TDREPORT - no TDX device available"
	EXIT_CODE=1
fi

# Test 5: Check kernel messages
echo ""
echo "Test 5: Checking TDX kernel messages..."
if dmesg | grep -i tdx | head -5 >/dev/null 2>&1; then
	echo "✅ TDX kernel messages found:"
	dmesg | grep -i tdx | head -5 | sed 's/^/   /'
else
	echo "⚠️  TDX kernel messages not accessible (may need sudo)"
fi

# Summary
echo ""
echo "=== TDX Verification Summary ==="
if [ $EXIT_CODE -eq 0 ]; then
	echo "✅ TDX environment is fully functional!"
	echo "   • TDX guest environment confirmed"
	echo "   • TDX device available and working"
	echo "   • TDREPORT generation successful"
	echo "   • TSM configfs interface available"
	echo ""
	echo "🎉 Ready for NXCC confidential computing development!"
else
	echo "❌ TDX environment has issues"
	echo "   Check the test results above for details"
	echo ""
	echo "This environment may not support TDX attestation properly."
fi

exit $EXIT_CODE
