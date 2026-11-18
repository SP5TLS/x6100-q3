// SPDX-License-Identifier: MIT
/*
 * x6100-btd: the X6100's Bluetooth pairing agent.
 *
 *   x6100-btd
 *
 * Registered with BlueZ as the default agent with NoInputNoOutput, so
 * keyboards, mice, headphones and speakers pair with "Just Works". It
 * says yes only while a pairing window is open: the file
 * /run/x6100/bt-pairing holds the time (seconds since the epoch) the
 * window closes; x6100-bt writes it for "pair" and "discoverable".
 * Outside the window every request from a device that is not paired and
 * trusted is refused, so nobody in range can pair with the radio unasked.
 * Devices paired here are marked trusted, so they reconnect without
 * asking.
 */

#include <gio/gio.h>
#include <glib-unix.h>
#include <signal.h>
#include <stdio.h>
#include <stdlib.h>
#include <time.h>
#include <unistd.h>

#define BLUEZ "org.bluez"
#define AGENT_PATH "/org/x6100/agent"
#define PAIRING_FILE "/run/x6100/bt-pairing"

static GDBusConnection *bus;
static GMainLoop *loop;
static guint agent_reg;

static const gchar introspection_xml[] =
	"<node>"
	"  <interface name='org.bluez.Agent1'>"
	"    <method name='Release'/>"
	"    <method name='RequestPinCode'>"
	"      <arg type='o' direction='in'/><arg type='s' direction='out'/>"
	"    </method>"
	"    <method name='DisplayPinCode'>"
	"      <arg type='o' direction='in'/><arg type='s' direction='in'/>"
	"    </method>"
	"    <method name='RequestPasskey'>"
	"      <arg type='o' direction='in'/><arg type='u' direction='out'/>"
	"    </method>"
	"    <method name='DisplayPasskey'>"
	"      <arg type='o' direction='in'/><arg type='u' direction='in'/>"
	"      <arg type='q' direction='in'/>"
	"    </method>"
	"    <method name='RequestConfirmation'>"
	"      <arg type='o' direction='in'/><arg type='u' direction='in'/>"
	"    </method>"
	"    <method name='RequestAuthorization'>"
	"      <arg type='o' direction='in'/>"
	"    </method>"
	"    <method name='AuthorizeService'>"
	"      <arg type='o' direction='in'/><arg type='s' direction='in'/>"
	"    </method>"
	"    <method name='Cancel'/>"
	"  </interface>"
	"</node>";

static GDBusNodeInfo *introspection;

/* ------------------------------------------------------------------------ */
/* Pairing window */

static gboolean pairing_open(void)
{
	gchar *text = NULL;
	gboolean open = FALSE;

	if (g_file_get_contents(PAIRING_FILE, &text, NULL, NULL)) {
		gint64 until = g_ascii_strtoll(text, NULL, 10);

		open = until > (gint64)time(NULL);
		g_free(text);
	}
	return open;
}

static GVariant *device_property(const gchar *device, const gchar *name)
{
	GVariant *reply, *value = NULL;

	reply = g_dbus_connection_call_sync(bus, BLUEZ, device,
					    "org.freedesktop.DBus.Properties", "Get",
					    g_variant_new("(ss)", "org.bluez.Device1", name),
					    G_VARIANT_TYPE("(v)"), G_DBUS_CALL_FLAGS_NONE,
					    2000, NULL, NULL);
	if (reply) {
		g_variant_get(reply, "(v)", &value);
		g_variant_unref(reply);
	}
	return value;
}

static gboolean device_trusted(const gchar *device)
{
	GVariant *v = device_property(device, "Trusted");
	gboolean trusted = v && g_variant_get_boolean(v);

	if (v)
		g_variant_unref(v);
	return trusted;
}

static void device_trust(const gchar *device)
{
	g_dbus_connection_call(bus, BLUEZ, device, "org.freedesktop.DBus.Properties", "Set",
			       g_variant_new("(ssv)", "org.bluez.Device1", "Trusted",
					     g_variant_new_boolean(TRUE)),
			       NULL, G_DBUS_CALL_FLAGS_NONE, 2000, NULL, NULL, NULL);
}

static void reject(GDBusMethodInvocation *inv, const gchar *device, const gchar *what)
{
	g_message("refused %s from %s (no pairing window)", what, device);
	g_dbus_method_invocation_return_dbus_error(inv, "org.bluez.Error.Rejected",
						   "Pairing is not open");
}

/* ------------------------------------------------------------------------ */
/* D-Bus objects */

static void method_call(GDBusConnection *c, const gchar *sender, const gchar *path,
			const gchar *iface, const gchar *method, GVariant *params,
			GDBusMethodInvocation *inv, gpointer data)
{
	const gchar *device = NULL;

	(void)c;
	(void)sender;
	(void)path;
	(void)iface;
	(void)data;

	if (g_variant_n_children(params) > 0)
		g_variant_get_child(params, 0, "&o", &device);

	if (!g_strcmp0(method, "RequestPinCode")) {
		/* Legacy (pre-2.1) devices: the usual fixed PIN. */
		if (!pairing_open()) {
			reject(inv, device, "PIN request");
			return;
		}
		device_trust(device);
		g_message("PIN 0000 for %s", device);
		g_dbus_method_invocation_return_value(inv, g_variant_new("(s)", "0000"));
	} else if (!g_strcmp0(method, "RequestPasskey")) {
		if (!pairing_open()) {
			reject(inv, device, "passkey request");
			return;
		}
		device_trust(device);
		g_dbus_method_invocation_return_value(inv, g_variant_new("(u)", 0));
	} else if (!g_strcmp0(method, "RequestConfirmation") ||
		   !g_strcmp0(method, "RequestAuthorization")) {
		if (!pairing_open()) {
			reject(inv, device, "pairing");
			return;
		}
		device_trust(device);
		g_message("paired with %s", device);
		g_dbus_method_invocation_return_value(inv, NULL);
	} else if (!g_strcmp0(method, "AuthorizeService")) {
		if (!device_trusted(device) && !pairing_open()) {
			reject(inv, device, "service");
			return;
		}
		g_dbus_method_invocation_return_value(inv, NULL);
	} else {
		/* Release, Cancel, Display*: nothing to show. */
		g_dbus_method_invocation_return_value(inv, NULL);
	}
}

static const GDBusInterfaceVTable vtable = { method_call, NULL, NULL, { 0 } };

static void call_bluez(const gchar *path, const gchar *iface, const gchar *method,
		       GVariant *args)
{
	GError *err = NULL;
	GVariant *r = g_dbus_connection_call_sync(bus, BLUEZ, path, iface, method, args, NULL,
						  G_DBUS_CALL_FLAGS_NONE, 5000, NULL, &err);

	if (r) {
		g_variant_unref(r);
	} else {
		g_warning("%s.%s: %s", iface, method, err->message);
		g_error_free(err);
	}
}

static void bluez_appeared(GDBusConnection *c, const gchar *name, const gchar *owner,
			   gpointer data)
{
	(void)c;
	(void)name;
	(void)owner;
	(void)data;

	call_bluez("/org/bluez", "org.bluez.AgentManager1", "RegisterAgent",
		   g_variant_new("(os)", AGENT_PATH, "NoInputNoOutput"));
	call_bluez("/org/bluez", "org.bluez.AgentManager1", "RequestDefaultAgent",
		   g_variant_new("(o)", AGENT_PATH));

	g_message("registered with bluetoothd as the pairing agent");
}

static void bluez_vanished(GDBusConnection *c, const gchar *name, gpointer data)
{
	(void)c;
	(void)name;
	(void)data;
	g_message("bluetoothd is gone; waiting for it");
}

static gboolean on_signal(gpointer data)
{
	(void)data;
	g_main_loop_quit(loop);
	return G_SOURCE_REMOVE;
}

int main(void)
{
	GError *err = NULL;

	bus = g_bus_get_sync(G_BUS_TYPE_SYSTEM, NULL, &err);
	if (!bus) {
		fprintf(stderr, "x6100-btd: system bus: %s\n", err->message);
		return 1;
	}
	introspection = g_dbus_node_info_new_for_xml(introspection_xml, NULL);
	agent_reg = g_dbus_connection_register_object(bus, AGENT_PATH,
						      introspection->interfaces[0], &vtable,
						      NULL, NULL, &err);
	if (!agent_reg) {
		fprintf(stderr, "x6100-btd: %s\n", err->message);
		return 1;
	}

	g_bus_watch_name_on_connection(bus, BLUEZ, G_BUS_NAME_WATCHER_FLAGS_NONE,
				       bluez_appeared, bluez_vanished, NULL, NULL);

	loop = g_main_loop_new(NULL, FALSE);
	g_unix_signal_add(SIGTERM, on_signal, NULL);
	g_unix_signal_add(SIGINT, on_signal, NULL);
	g_main_loop_run(loop);
	return 0;
}
