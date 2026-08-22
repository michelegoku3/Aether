#!/usr/bin/env python3
# ============================================================================
# achievement_decoder.py — decoder/encoder dei file cache achievement di Steam
# + gestione dei backup snapshot JSON creati da AetherDLL.
#
# COMANDI
#
#   1) Decodifica (default) — passagli uno o piu' file:
#
#        python achievement_decoder.py UserGameStatsSchema_1817070.bin \
#                                     UserGameStats_1916127674_1817070.bin
#
#      Decodifica .bin di schema/stats (e anche gli snapshot .json del backup
#      AetherDLL) e stampa la timeline degli sblocchi con i nomi. Lo schema e'
#      necessario solo per tradurre i bit in nomi.
#
#   2) snapshot — converte un .bin di Steam nello stesso formato JSON usato dal
#      backup di AetherDLL (utile per "seminare" il backup con gli achievement
#      gia' sbloccati PRIMA di installare la nuova build):
#
#        python achievement_decoder.py snapshot UserGameStats_1916127674_1817070.bin \
#                --schema UserGameStatsSchema_1817070.bin \
#                -o UserGameStats_1916127674_1817070.json
#
#   3) rebuild — ricostruisce un .bin valido per Steam a partire dallo snapshot
#      JSON del backup (da rimettere in <steam>\appcache\stats\ con Steam
#      chiuso per far riapparire gli achievement):
#
#        python achievement_decoder.py rebuild UserGameStats_1916127674_1817070.json \
#                --schema UserGameStatsSchema_1817070.bin \
#                -o UserGameStats_1916127674_1817070.bin
#
# FORMATO INTERNO (binary VDF di Valve)
#   0x00 = apertura dizionario (seguita da chiave\0)
#   0x01 = stringa  (chiave\0 valore\0)
#   0x02 = int32    (chiave\0 + 4 byte little-endian)
#   0x03 = float32  (chiave\0 + 4 byte)
#   0x08 = chiusura dizionario
#
# Struttura di UserGameStats_<account>_<appid>.bin:
#   cache { crc=0, PendingChanges>0 se ci sono cambi non confermati dal server,
#           <bucket> { data=bitfield32, state, AchievementTimes { <bit>: unix } } }
#
# CONVENZIONE ID <-> BUCKET/BIT:  achievement_id (protocollo eMsg 5466) =
# bucket*32 + bit. Il comando snapshot deriva gli id cosi'; il rebuild ricontrola
# i bucket/bit contro lo schema quando fornito (warn se un bucket non esiste).
# ============================================================================

import struct
import sys
import json
import argparse
from datetime import datetime, timezone, timedelta

# Ora locale italiana (CEST) per la visualizzazione; cambia se ti serve.
LOCAL_TZ = timezone(timedelta(hours=2))

MIRROR_FORMAT = 'aether-achievement-mirror-v1'


# ---------------------------------------------------------------------------
# Parsing binary VDF
# ---------------------------------------------------------------------------

def parse_bin_vdf(buf, pos=0):
    """Parser binary VDF. Ritorna (lista di (chiave, valore), nuova_pos)."""
    out = []
    while pos < len(buf):
        t = buf[pos]
        pos += 1
        if t == 0x08:
            return out, pos
        end = buf.index(b'\x00', pos)
        key = buf[pos:end].decode('utf-8', 'replace')
        pos = end + 1
        if t == 0x00:
            val, pos = parse_bin_vdf(buf, pos)
            out.append((key, dict(val)))
        elif t == 0x01:
            end = buf.index(b'\x00', pos)
            out.append((key, ('str', bytes(buf[pos:end]))))
            pos = end + 1
        elif t == 0x02:
            val = struct.unpack('<i', buf[pos:pos + 4])[0]
            out.append((key, ('int', val)))
            pos += 4
        elif t == 0x03:
            val = struct.unpack('<f', buf[pos:pos + 4])[0]
            out.append((key, ('flt', val)))
            pos += 4
        else:
            raise ValueError(f'tipo sconosciuto 0x{t:02x} alla posizione {pos - 1}')
    return out, pos


def fmt_time(unix_ts):
    try:
        return datetime.fromtimestamp(unix_ts, LOCAL_TZ).strftime('%d/%m/%Y %H:%M:%S')
    except (OverflowError, OSError, ValueError):
        return '?'


# ---------------------------------------------------------------------------
# Lettura: schema / stats bin / snapshot json
# ---------------------------------------------------------------------------

def parse_schema(data):
    """Ritorna ({bucket: {bit: (api, english)}}, meta)."""
    root = dict(parse_bin_vdf(data)[0])
    appid_key = next(iter(root))
    app = root[appid_key]
    schema = {}
    for bucket, bv in app.get('stats', {}).items():
        bits = bv.get('bits', {})
        if not isinstance(bits, dict):
            continue
        for bit, v in bits.items():
            if not isinstance(v, dict):
                continue
            nm = v.get('name', ('str', b'?'))[1]
            eng = ''
            disp = v.get('display', {})
            if isinstance(disp, dict):
                names = disp.get('name', {})
                if isinstance(names, dict):
                    eng_b = names.get('english', ('str', b''))[1]
                    eng = eng_b.decode('utf-8', 'replace')
            schema.setdefault(int(bucket), {})[int(bit)] = (
                nm.decode('utf-8', 'replace') if isinstance(nm, bytes) else str(nm), eng)
    meta = {'appid': appid_key}
    for k, attr in (('gamename', 'gamename'), ('version', 'version')):
        if k in app:
            val = app[k][1]
            meta[attr] = val.decode('utf-8', 'replace') if isinstance(val, bytes) else val
    return schema, meta


def parse_stats(data):
    """Ritorna (crc, pending, {bucket: {'data': int, 'times': {bit: unix}}})."""
    root = dict(parse_bin_vdf(data)[0])
    cache = root.get('cache', {})
    crc = cache.get('crc', ('int', 0))[1]
    pending = cache.get('PendingChanges', ('int', 0))[1]
    buckets = {}
    for bucket, bv in cache.items():
        if not isinstance(bv, dict) or 'AchievementTimes' not in bv:
            continue
        bitfield = bv.get('data', ('int', 0))[1] & 0xFFFFFFFF
        times = {}
        for bit, tv in bv['AchievementTimes'].items():
            if isinstance(tv, tuple) and tv[0] == 'int':
                times[int(bit)] = tv[1]
        buckets[int(bucket)] = {'data': bitfield, 'times': times}
    return crc, pending, buckets


def parse_snapshot(data):
    """Ritorna (meta, [(id, unlock_time, bucket, bit), ...]) da uno snapshot JSON."""
    obj = json.loads(data.decode('utf-8'))
    if str(obj.get('_format', '')).startswith('aether-achievement-mirror'):
        entries = obj.get('achievements', [])
    elif 'achievements' in obj:
        entries = obj['achievements']
    else:
        raise ValueError('JSON senza chiave "achievements"')
    rows = []
    for e in entries:
        eid = int(e['id'])
        t = int(e.get('unlock_time', 0))
        bucket = e.get('bucket')
        bit = e.get('bit')
        if bucket is None or bit is None:
            bucket, bit = eid // 32, eid % 32
        rows.append((eid, t, int(bucket), int(bit)))
    return obj, rows


# ---------------------------------------------------------------------------
# Scrittura: snapshot json / bin
# ---------------------------------------------------------------------------

def now_iso():
    return datetime.now(LOCAL_TZ).strftime('%Y-%m-%dT%H:%M:%S')


def snapshot_json(appid, account_id, steamid64, entries):
    """entries: [(id, unlock_time)] ordinato per id."""
    out = ['{',
           f'  "_format": "{MIRROR_FORMAT}",',
           '  "_note": "snapshot generato da AetherDLL; ricostruisci il .bin con Tools/achievement_decoder.py rebuild",',
           f'  "appid": {appid},',
           f'  "account_id": {account_id},',
           f'  "steamid64": {steamid64},',
           f'  "updated": "{now_iso()}",',
           '  "achievements": [']
    for i, (eid, t) in enumerate(entries):
        out.append(f'    {{"id": {eid}, "unlock_time": {t}, "unlocked_at": "{fmt_time(t)}", '
                   f'"bucket": {eid // 32}, "bit": {eid % 32}}}' + (',' if i + 1 < len(entries) else ''))
    out.append('  ]')
    out.append('}')
    return ('\n'.join(out) + '\n').encode('utf-8')


def build_bin(buckets, pending=1):
    """buckets: {bucket: {bit: unix_time}} -> byte del file UserGameStats."""
    out = bytearray()
    out += b'\x00cache\x00'
    out += b'\x02crc\x00' + struct.pack('<i', 0)
    out += b'\x02PendingChanges\x00' + struct.pack('<i', pending)
    for bucket in sorted(buckets):
        times = buckets[bucket]
        bitfield = 0
        for bit in times:
            bitfield |= (1 << bit)
        signed = bitfield if bitfield < (1 << 31) else bitfield - (1 << 32)
        out += b'\x00' + str(bucket).encode() + b'\x00'
        out += b'\x02data\x00' + struct.pack('<i', signed)
        out += b'\x02state\x00' + struct.pack('<i', 2)
        out += b'\x00AchievementTimes\x00'
        for bit in sorted(times):
            out += b'\x02' + str(bit).encode() + b'\x00' + struct.pack('<i', times[bit])
        out += b'\x08'   # chiude AchievementTimes
        out += b'\x08'   # chiude il bucket
    out += b'\x08'       # chiude cache
    out += b'\x08'       # chiude il dizionario radice (scritto da Steam stesso)
    return bytes(out)


# ---------------------------------------------------------------------------
# Comandi
# ---------------------------------------------------------------------------

def cmd_decode(paths):
    schema = {}
    schema_meta = {}
    items = []   # ('stats_bin', path, data) | ('snapshot', path, data)

    for path in paths:
        data = open(path, 'rb').read()
        try:
            entries, _ = parse_bin_vdf(data)
            keys = [k for k, _ in entries]
        except ValueError as e:
            if path.lower().endswith('.json'):
                try:
                    meta, rows = parse_snapshot(data)
                    items.append(('snapshot', path, (meta, rows)))
                    print(f'[snapshot] {path}: {len(rows)} achievement '
                          f'(appid {meta.get("appid", "?")}, account {meta.get("account_id", "?")})')
                    continue
                except Exception as je:
                    print(f'[!] {path}: parse failed ({e} / {je})')
                    continue
            print(f'[!] {path}: parse failed ({e})')
            continue
        if len(keys) == 1 and keys[0].isdigit():          # schema: radice = appid
            schema, schema_meta = parse_schema(data)
            print(f'[schema] {path}: appid {schema_meta.get("appid")}, '
                  f'internal game name "{schema_meta.get("gamename", "?")}", '
                  f'{sum(len(v) for v in schema.values())} achievement in {len(schema)} bucket')
        elif 'cache' in keys:                             # file UserGameStats
            items.append(('stats_bin', path, data))
        else:
            print(f'[!] {path}: unrecognized structure (keys: {keys[:3]})')

    for kind, path, payload in items:
        print()
        print(f'===== {path} =====')
        if kind == 'stats_bin':
            crc, pending, buckets = parse_stats(payload)
            print(f'  crc = {crc}, PendingChanges = {pending} '
                  f'({"local changes NOT confirmed by the server" if pending else "all confirmed"})')
            print(f'  bucket: {sorted(buckets.keys())}')
            rows = []
            total = 0
            for bucket, info in sorted(buckets.items()):
                names = schema.get(bucket, {})
                for bit, t in sorted(info['times'].items()):
                    rows.append((t, bucket, bit, bucket * 32 + bit, names.get(bit, ('?', '(bit not defined in schema)'))))
                total += bin(info['data']).count('1')
            print(f'  achievements recorded in this file: {total}')
        else:
            meta, rows_raw = payload
            print(f'  AetherDLL backup snapshot — account {meta.get("account_id")}, '
                  f'appid {meta.get("appid")}, updated {meta.get("updated", "?")}')
            rows = []
            for eid, t, bucket, bit in rows_raw:
                names = schema.get(bucket, {})
                rows.append((t, bucket, bit, eid, names.get(bit, ('?', '(bucket/bit not in schema)'))))
            print(f'  achievements recorded in this file: {len(rows)}')

        if rows:
            print('  --- Unlock timeline (id = bucket*32+bit) ---')
            for t, bucket, bit, eid, (api, eng) in sorted(rows):
                print(f'  {fmt_time(t)}  bucket={bucket:3d} bit={bit:2d} id={eid:5d}  {api:34s} "{eng}"')
        else:
            print('  NO achievements in this file (empty cache).')
    return 0


def cmd_snapshot(args):
    data = open(args.bin, 'rb').read()
    crc, pending, buckets = parse_stats(data)

    # account/appid dal nome file UserGameStats_<account>_<appid>.bin
    import os
    base = os.path.basename(args.bin)
    account_id, appid = '?', '?'
    if base.startswith('UserGameStats_'):
        parts = base[len('UserGameStats_'):].split('.')[0].split('_')
        if len(parts) >= 2:
            account_id, appid = parts[0], parts[1]

    entries = []
    for bucket, info in sorted(buckets.items()):
        for bit, t in sorted(info['times'].items()):
            entries.append((bucket * 32 + bit, t))
    entries.sort()
    steamid64 = 0x0110000100000000 | int(account_id) if str(account_id).isdigit() else 0
    blob = snapshot_json(appid, account_id, steamid64, entries)
    if args.output:
        open(args.output, 'wb').write(blob)
        print(f'[ok] wrote {len(entries)} achievements -> {args.output}')
    else:
        sys.stdout.write(blob.decode('utf-8'))
    return 0


def cmd_rebuild(args):
    meta, rows = parse_snapshot(open(args.json_file, 'rb').read())
    schema = {}
    if args.schema:
        schema, _ = parse_schema(open(args.schema, 'rb').read())

    buckets = {}
    for eid, t, bucket, bit in rows:
        if schema and bucket not in schema:
            print(f'[!] WARN: bucket {bucket} (da id {eid}) non esiste nello schema — lo scrivo comunque.')
        buckets.setdefault(bucket, {})[bit] = t

    blob = build_bin(buckets, pending=args.pending)
    out = args.output or ('UserGameStats_%s_%s.bin' % (meta.get('account_id', 'X'), meta.get('appid', 'X')))
    open(out, 'wb').write(blob)
    print(f'[ok] rebuilt {out}: {len(rows)} achievements in {len(buckets)} buckets, '
          f'PendingChanges={args.pending}')
    print('     To restore: close Steam, copy the file into <steam>\\appcache\\stats\\, restart Steam.')
    return 0


def main(argv):
    ap = argparse.ArgumentParser(description='Steam achievement cache decoder/encoder + AetherDLL backup tool')
    sub = ap.add_subparsers(dest='command')

    p_dec = sub.add_parser('decode', help='decode bin/schema/json files (default)')
    p_dec.add_argument('files', nargs='+')

    p_snap = sub.add_parser('snapshot', help='UserGameStats .bin -> snapshot JSON (AetherDLL backup format)')
    p_snap.add_argument('bin')
    p_snap.add_argument('--schema')
    p_snap.add_argument('-o', '--output')

    p_reb = sub.add_parser('rebuild', help='snapshot JSON -> UserGameStats .bin')
    p_reb.add_argument('json_file')
    p_reb.add_argument('--schema', help='schema .bin to validate bucket/bit (recommended)')
    p_reb.add_argument('--pending', type=int, default=1, help='PendingChanges value written into the .bin (default 1)')
    p_reb.add_argument('-o', '--output')

    # Modalita' default: senza sottocomando, tutti gli argomenti sono file da decodificare.
    args = ap.parse_args(argv[1:] if argv[1:2] and argv[1] in ('decode', 'snapshot', 'rebuild')
                         else ['decode'] + argv[1:])

    if args.command == 'decode':
        return cmd_decode(args.files)
    if args.command == 'snapshot':
        return cmd_snapshot(args)
    if args.command == 'rebuild':
        return cmd_rebuild(args)
    ap.print_help()
    return 1


if __name__ == '__main__':
    sys.exit(main(sys.argv))
