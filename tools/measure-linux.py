#!/usr/bin/env python3
"""Richtwert-Messung der PC-App unter Linux (Xvfb + WebKitGTK statt Windows + WebView2).

Echte Zahlen liefert nur tools/measure.ps1 unter Windows. Dieses Skript dient dazu, in der
Entwicklungsumgebung Regressionen zu erkennen (RAM/CPU im Leerlauf, mit Overlay, nach dem Schließen).

Aufruf: python3 tools/measure-linux.py pc-app/target/release/panco
"""
import os, subprocess, sys, tempfile, time

CLK = os.sysconf("SC_CLK_TCK")


def tree(root):
    """PID + alle Nachfahren."""
    kids = {}
    for p in os.listdir("/proc"):
        if p.isdigit():
            try:
                stat = open(f"/proc/{p}/stat").read()
                ppid = int(stat[stat.rindex(")") + 2 :].split()[1])
                kids.setdefault(ppid, []).append(int(p))
            except OSError:
                pass
    out, todo = [], [root]
    while todo:
        p = todo.pop()
        out.append(p)
        todo += kids.get(p, [])
    return out


def sample(root):
    """(USS in MB, CPU-Sekunden, Prozessnamen). USS = privater Speicher, vergleichbar mit
    "Arbeitsspeicher (privater Arbeitssatz)" im Windows-Task-Manager."""
    rss, cpu, names = 0, 0.0, []
    for p in tree(root):
        try:
            for line in open(f"/proc/{p}/smaps_rollup"):
                if line.startswith(("Private_Clean:", "Private_Dirty:")):
                    rss += int(line.split()[1])
            stat = open(f"/proc/{p}/stat").read()
            f = stat[stat.rindex(")") + 2 :].split()
            cpu += (int(f[11]) + int(f[12])) / CLK
            names.append(stat[stat.index("(") + 1 : stat.rindex(")")])
        except OSError:
            pass
    return rss / 1024, cpu, names


def measure(root, label, seconds):
    _, c0, _ = sample(root)
    t0 = time.time()
    time.sleep(seconds)
    rss, c1, names = sample(root)
    pct = (c1 - c0) / (time.time() - t0) * 100
    print(f"  {label:<34} RAM (USS) {rss:7.1f} MB   CPU {pct:5.2f} % eines Kerns   Prozesse: {', '.join(sorted(set(names)))}")
    return rss, pct


def main():
    exe = os.path.abspath(sys.argv[1])
    secs = int(os.environ.get("SECS", "20"))
    home = tempfile.mkdtemp(prefix="panco-home-")
    env = dict(os.environ, HOME=home, XDG_CONFIG_HOME=f"{home}/.config", DISPLAY=":99", NO_AT_BRIDGE="1")
    xvfb = subprocess.Popen(["Xvfb", ":99", "-screen", "0", "1920x1080x24"], stderr=subprocess.DEVNULL)
    time.sleep(1)
    dbus = subprocess.run(
        ["dbus-daemon", "--session", "--fork", "--print-address"], capture_output=True, text=True, env=env
    ).stdout.strip()
    env["DBUS_SESSION_BUS_ADDRESS"] = dbus
    if os.environ.get("REDUCED") == "1":
        os.makedirs(f"{home}/.config/Panco", exist_ok=True)
        with open(f"{home}/.config/Panco/config.json", "w") as f:
            f.write('{"overlay": {"reduce_motion": true}}')
    app = subprocess.Popen([exe], env=env, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)
    try:
        time.sleep(4)
        mode = "reduzierte Animationen" if os.environ.get("REDUCED") == "1" else "Standard"
        print(f"Panco unter Linux/Xvfb, Software-Rendering (Richtwert, nicht Windows!) – {secs} s je Phase, {mode}")
        measure(app.pid, "Leerlauf (Tray, kein Fenster)", secs)
        subprocess.run(["xdotool", "key", "ctrl+space"], env=env)
        time.sleep(3)
        measure(app.pid, "Overlay offen (Avatar animiert)", secs)
        # Schließen per Hotkey (Escape erreicht im Xvfb ohne Fenstermanager das Fenster nicht).
        subprocess.run(["xdotool", "key", "ctrl+space"], env=env)
        time.sleep(3)
        measure(app.pid, "Overlay wieder geschlossen", secs)
    finally:
        app.terminate()
        app.wait(5)
        xvfb.terminate()
        err = app.stderr.read().decode(errors="replace").strip()
        if err:
            print("\nstderr:", err[:800])


if __name__ == "__main__":
    main()
