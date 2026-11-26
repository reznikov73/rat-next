#!/usr/bin/env python3
"""Функциональная проверка relay: pairing по ID и пересылка байтов.

Эмулирует то, что делают настоящие клиенты Rat Next (crates/session/src/link.rs):
агент регистрируется командой `REG <id> <name>`, вьюер просит `CONNECT <id>`,
после чего relay переводит оба сокета в «сырой» режим и перекачивает байты.

Проверяем:
  1. `WHO` для незарегистрированного ID → ERR OFFLINE;
  2. регистрация агента → OK;
  3. `WHO` для зарегистрированного ID → OK;
  4. `CONNECT` вьюера → OK у вьюера и `PAIRED` у агента;
  5. байты, отправленные вьюером, доходят до агента (и наоборот);
  6. `CONNECT` к неизвестному ID → ERR OFFLINE.

Запуск: python3 scripts/relay-smoke.py <host:port>
"""

import socket
import sys
import time


def connect(addr):
    host, port = addr.rsplit(":", 1)
    s = socket.create_connection((host, int(port)), timeout=5)
    s.settimeout(5)
    return s


def read_line(s):
    """Прочитать строку до \n (протокол построчный)."""
    buf = b""
    while not buf.endswith(b"\n"):
        ch = s.recv(1)
        if not ch:
            raise AssertionError(f"соединение закрыто, получено: {buf!r}")
        buf += ch
    return buf.decode("utf-8", "replace").strip()


def send_line(s, line):
    s.sendall((line + "\n").encode())


def main():
    addr = sys.argv[1] if len(sys.argv) > 1 else "127.0.0.1:18889"
    session_id = "SMOKE-1"
    checks = []

    def check(name, cond, detail=""):
        checks.append((name, cond, detail))
        mark = "OK  " if cond else "FAIL"
        print(f"  [{mark}] {name}" + (f" — {detail}" if detail else ""))

    print(f"relay smoke: {addr}")

    # 1. WHO для незарегистрированного ID
    who = connect(addr)
    send_line(who, f"WHO {session_id}")
    reply = read_line(who)
    check("WHO неизвестного ID → ERR OFFLINE", reply == "ERR OFFLINE", reply)
    who.close()

    # 2. Регистрация агента
    agent = connect(addr)
    send_line(agent, f"REG {session_id} smoke-agent")
    reply = read_line(agent)
    check("REG агента → OK", reply == "OK", reply)

    # 3. WHO для зарегистрированного ID
    who = connect(addr)
    send_line(who, f"WHO {session_id}")
    reply = read_line(who)
    check("WHO известного ID → OK", reply == "OK", reply)
    who.close()

    # 4. CONNECT вьюера
    viewer = connect(addr)
    send_line(viewer, f"CONNECT {session_id}")
    reply = read_line(viewer)
    check("CONNECT вьюера → OK", reply == "OK", reply)

    paired = read_line(agent)
    check("агент получил PAIRED", paired == "PAIRED", paired)

    # 5. Пересылка байтов в обе стороны
    viewer.settimeout(5)
    agent.settimeout(5)

    viewer.sendall(b"viewer->agent")
    got = agent.recv(64)
    check("байты вьюер → агент", got == b"viewer->agent", repr(got))

    agent.sendall(b"agent->viewer")
    got = viewer.recv(64)
    check("байты агент → вьюер", got == b"agent->viewer", repr(got))

    # Бинарные данные, включая нули и \n — relay не должен их разбирать.
    payload = bytes(range(256)) + b"\n\n" + b"\x00" * 16
    viewer.sendall(payload)
    received = b""
    while len(received) < len(payload):
        chunk = agent.recv(4096)
        if not chunk:
            break
        received += chunk
    check(
        "бинарные данные (256 байт + переводы строк) без искажений",
        received == payload,
        f"получено {len(received)} из {len(payload)}",
    )

    viewer.close()
    agent.close()

    # 6. CONNECT к неизвестному ID
    time.sleep(0.2)
    bad = connect(addr)
    send_line(bad, "CONNECT NOBODY")
    reply = read_line(bad)
    check("CONNECT неизвестного ID → ERR OFFLINE", reply == "ERR OFFLINE", reply)
    bad.close()

    failed = [c for c in checks if not c[1]]
    print()
    if failed:
        print(f"ПРОВАЛ: {len(failed)} из {len(checks)}")
        return 1
    print(f"УСПЕХ: {len(checks)} из {len(checks)} проверок пройдено")
    return 0


if __name__ == "__main__":
    sys.exit(main())
