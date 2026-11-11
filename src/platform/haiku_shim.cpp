// What only Haiku's C++ kits can tell: the graphics card and screen mode (app_server, which
// BScreen reaches through a BApplication) and the joined Wi-Fi network (BNetworkDevice).

#include <Application.h>
#include <NetworkDevice.h>
#include <NetworkInterface.h>
#include <NetworkRoster.h>
#include <Screen.h>

#include <string.h>

// Bit 1: gpu was filled. Bit 2: screen[0..3] (width, height, refresh in mHz) and screenName were
extern "C" int
haikufetch_graphics(char* gpu, size_t gpuSize, char* screenName, size_t screenNameSize,
	uint32* screen)
{
	status_t status;
	BApplication app("application/x-vnd.haikufetch", &status);
	if (status != B_OK)
		return 0;

	BScreen mainScreen(B_MAIN_SCREEN_ID);
	if (!mainScreen.IsValid())
		return 0;

	int found = 0;
	// The chipset is what the accelerant knows the card as, its name is the driver's
	accelerant_device_info device;
	if (mainScreen.GetDeviceInfo(&device) == B_OK) {
		strlcpy(gpu, device.chipset[0] != '\0' ? device.chipset : device.name, gpuSize);
		found |= 1;
	}

	display_mode mode;
	if (mainScreen.GetMode(&mode) == B_OK) {
		screen[0] = mode.virtual_width;
		screen[1] = mode.virtual_height;
		// pixel_clock is in kHz, so this is the refresh rate in mHz
		uint64 pixels = (uint64)mode.timing.h_total * mode.timing.v_total;
		screen[2] = pixels > 0 ? (uint32)((uint64)mode.timing.pixel_clock * 1000000 / pixels) : 0;
		found |= 2;

		monitor_info monitor;
		if (mainScreen.GetMonitorInfo(&monitor) == B_OK)
			strlcpy(screenName, monitor.name, screenNameSize);
	}
	return found;
}

// The name of the first network a wireless interface is joined to. 1 if found
extern "C" int
haikufetch_wifi(char* ssid, size_t ssidSize)
{
	BNetworkRoster& roster = BNetworkRoster::Default();
	BNetworkInterface interface;
	uint32 cookie = 0;
	while (roster.GetNextInterface(&cookie, interface) == B_OK) {
		BNetworkDevice device(interface.Name());
		if (!device.IsWireless())
			continue;
		uint32 networkCookie = 0;
		wireless_network network;
		if (device.GetNextAssociatedNetwork(networkCookie, network) == B_OK && network.name[0]) {
			strlcpy(ssid, network.name, ssidSize);
			return 1;
		}
	}
	return 0;
}
