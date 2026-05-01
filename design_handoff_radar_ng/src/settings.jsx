// Radar-NG — Settings screen (Data Sources focus for self-hosted stack)

function SettingsScreen({ onBack, settings, onChange }) {
  const t = window.CUMULUS.tokens;
  const f = window.CUMULUS.fonts;
  const s = settings;

  const upd = (k, v) => onChange({ ...s, [k]: v });
  const updSource = (sourceKey, patch) => onChange({ ...s, sources: { ...s.sources, [sourceKey]: { ...s.sources[sourceKey], ...patch } } });

  return (
    <PullToRefresh accent="#8B7CFF" onRefresh={() => new Promise(r => setTimeout(r, 600))}>
    <div style={{ width: '100%', minHeight: '100%', background: '#0a0e1a', color: '#fff', fontFamily: f.ui, paddingTop: 56, paddingBottom: 100 }}>

      {/* Header */}
      <div style={{ padding: '10px 16px 6px', display: 'flex', alignItems: 'center', gap: 10 }}>
        <div onClick={onBack} style={{ width: 36, height: 36, borderRadius: 18, background: 'rgba(255,255,255,0.08)', display: 'flex', alignItems: 'center', justifyContent: 'center', cursor: 'pointer' }}>
          <svg width="14" height="14" viewBox="0 0 14 14"><path d="M9 2 L4 7 L9 12" stroke="#fff" strokeWidth="1.8" fill="none" strokeLinecap="round" strokeLinejoin="round"/></svg>
        </div>
        <div style={{ fontSize: 28, fontWeight: 700, letterSpacing: -0.5 }}>Settings</div>
      </div>

      {/* Hosted stack summary */}
      <div style={{ margin: '14px 16px 10px', padding: 14, borderRadius: 18,
        background: 'linear-gradient(135deg, rgba(139,124,255,0.18), rgba(79,184,255,0.12))',
        border: '1px solid rgba(139,124,255,0.3)' }}>
        <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', marginBottom: 8 }}>
          <div style={{ fontSize: 11, fontFamily: f.mono, letterSpacing: '0.12em', color: '#C7BDFF', fontWeight: 700 }}>SELF-HOSTED STACK</div>
          <StatusDot ok />
        </div>
        <div style={{ fontSize: 15, fontWeight: 600, marginBottom: 2 }}>{s.stackName}</div>
        <div style={{ fontSize: 12, color: 'rgba(255,255,255,0.6)', fontFamily: f.mono }}>{s.stackUrl}</div>
        <div style={{ display: 'grid', gridTemplateColumns: 'repeat(3, 1fr)', gap: 10, marginTop: 12 }}>
          <Stat label="UPTIME" value="14d 6h" />
          <Stat label="TILES/DAY" value="2.4M" />
          <Stat label="CACHE" value="87%" />
        </div>
      </div>

      <SectionHeader>Data Sources</SectionHeader>
      <Card>
        <SourceRow
          icon="🌩"
          name="Radar tiles (MRMS)"
          endpoint={s.sources.radar.url}
          status={s.sources.radar.status}
          onEdit={() => upd('editing', 'radar')}
        />
        <Sep />
        <SourceRow
          icon="🛰"
          name="Satellite (GOES-18)"
          endpoint={s.sources.satellite.url}
          status={s.sources.satellite.status}
          onEdit={() => upd('editing', 'satellite')}
        />
        <Sep />
        <SourceRow
          icon="🌡"
          name="Forecast model (HRRR)"
          endpoint={s.sources.forecast.url}
          status={s.sources.forecast.status}
          onEdit={() => upd('editing', 'forecast')}
        />
        <Sep />
        <SourceRow
          icon="🗺"
          name="Base map tiles"
          endpoint={s.sources.basemap.url}
          status={s.sources.basemap.status}
          onEdit={() => upd('editing', 'basemap')}
        />
        <Sep />
        <SourceRow
          icon="⚠️"
          name="Alerts (NWS CAP)"
          endpoint={s.sources.alerts.url}
          status={s.sources.alerts.status}
          onEdit={() => upd('editing', 'alerts')}
        />
      </Card>

      {/* Inline editor for selected source */}
      {s.editing && <SourceEditor sourceKey={s.editing} source={s.sources[s.editing]} updSource={updSource} close={() => upd('editing', null)} />}

      <SectionHeader>Docker Stack</SectionHeader>
      <Card>
        <Row>
          <RowLeft title="Container runtime" sub="docker compose v2.24" />
          <Pill color="#4ADE80">Running</Pill>
        </Row>
        <Sep />
        <ContainerRow name="radar-ng-mrms" image="ghcr.io/radar-ng/mrms:1.4.2" status="healthy" ports="8081→80" />
        <Sep />
        <ContainerRow name="radar-ng-goes" image="ghcr.io/radar-ng/goes:0.9.1" status="healthy" ports="8082→80" />
        <Sep />
        <ContainerRow name="radar-ng-hrrr" image="ghcr.io/radar-ng/hrrr:2.1.0" status="updating" ports="8083→80" />
        <Sep />
        <ContainerRow name="radar-ng-tiles" image="ghcr.io/maptiler/osm:latest" status="healthy" ports="8084→80" />
        <Sep />
        <ContainerRow name="radar-ng-proxy" image="caddy:2.7" status="healthy" ports="443→443" />
        <Sep />
        <ContainerRow name="radar-ng-cache" image="varnish:7.4" status="healthy" ports="6081→6081" />
      </Card>

      <div style={{ padding: '10px 16px 0', display: 'flex', gap: 10 }}>
        <BigBtn primary>Pull & Restart</BigBtn>
        <BigBtn>View Logs</BigBtn>
      </div>

      <SectionHeader>Tile Cache</SectionHeader>
      <Card>
        <Row>
          <RowLeft title="Storage" sub="1.8 GB of 4 GB used" />
          <div style={{ fontSize: 13, fontFamily: f.mono, color: 'rgba(255,255,255,0.5)' }}>45%</div>
        </Row>
        <div style={{ padding: '0 14px 12px' }}>
          <div style={{ height: 4, background: 'rgba(255,255,255,0.08)', borderRadius: 2, overflow: 'hidden' }}>
            <div style={{ width: '45%', height: '100%', background: '#8B7CFF', borderRadius: 2 }} />
          </div>
        </div>
        <Sep />
        <ToggleRow label="Preload nearby tiles" sub="Pre-cache ±25 mi for offline viewing" value={s.preload} onChange={v => upd('preload', v)} />
        <Sep />
        <ToggleRow label="Share to CDN" sub="Cloudflare R2 edge cache" value={s.cdn} onChange={v => upd('cdn', v)} />
        <Sep />
        <Row>
          <RowLeft title="Purge cache" sub="Next auto-purge in 2d 14h" />
          <div style={{ fontSize: 13, color: '#FF6E7A', fontWeight: 600, cursor: 'pointer' }}>Clear</div>
        </Row>
      </Card>

      <SectionHeader>Network</SectionHeader>
      <Card>
        <SelectRow label="Refresh interval" value={s.refresh} onChange={v => upd('refresh', v)} options={['30 sec', '1 min', '2 min', '5 min']} />
        <Sep />
        <ToggleRow label="Use cellular for tiles" value={s.cellular} onChange={v => upd('cellular', v)} />
        <Sep />
        <ToggleRow label="VPN to home network" sub="Tailscale · 100.64.12.1" value={s.vpn} onChange={v => upd('vpn', v)} />
        <Sep />
        <Row>
          <RowLeft title="API rate limit" sub="600 req/min" />
          <div style={{ fontSize: 13, fontFamily: f.mono, color: 'rgba(255,255,255,0.5)' }}>12/600</div>
        </Row>
      </Card>

      <SectionHeader>About</SectionHeader>
      <Card>
        <Row><RowLeft title="App version" /><Mono>1.2.0 (420)</Mono></Row>
        <Sep />
        <Row><RowLeft title="Server version" /><Mono>1.3.1</Mono></Row>
        <Sep />
        <Row><RowLeft title="Last sync" /><Mono>34s ago</Mono></Row>
      </Card>

      <div style={{ padding: '16px 16px 40px', textAlign: 'center', fontSize: 11, color: 'rgba(255,255,255,0.4)', fontFamily: f.mono }}>
        Radar-NG is open source. Self-hosted, offline-capable.
      </div>
    </div>
    </PullToRefresh>
  );
}

function StatusDot({ ok }) {
  return (
    <div style={{ display: 'flex', alignItems: 'center', gap: 6, fontSize: 11, fontFamily: window.CUMULUS.fonts.mono, color: ok ? '#4ADE80' : '#FF6E7A', fontWeight: 600, letterSpacing: '0.04em' }}>
      <div style={{ width: 7, height: 7, borderRadius: 4, background: ok ? '#4ADE80' : '#FF6E7A', boxShadow: `0 0 8px ${ok ? '#4ADE80' : '#FF6E7A'}` }} />
      {ok ? 'ONLINE' : 'ERROR'}
    </div>
  );
}

function Stat({ label, value }) {
  const f = window.CUMULUS.fonts;
  return (
    <div>
      <div style={{ fontSize: 9, fontFamily: f.mono, color: 'rgba(255,255,255,0.5)', letterSpacing: '0.1em', fontWeight: 600 }}>{label}</div>
      <div style={{ fontSize: 15, fontWeight: 600, marginTop: 2 }}>{value}</div>
    </div>
  );
}

function SectionHeader({ children }) {
  return (
    <div style={{ padding: '18px 20px 6px', fontSize: 10, fontFamily: window.CUMULUS.fonts.mono, color: 'rgba(255,255,255,0.4)', letterSpacing: '0.14em', fontWeight: 700, textTransform: 'uppercase' }}>{children}</div>
  );
}

function Card({ children }) {
  return <div style={{ margin: '0 14px', background: 'rgba(255,255,255,0.05)', border: '1px solid rgba(255,255,255,0.06)', borderRadius: 16, overflow: 'hidden' }}>{children}</div>;
}

function Sep() { return <div style={{ height: 0.5, background: 'rgba(255,255,255,0.07)', marginLeft: 14 }} />; }

function Row({ children }) {
  return <div style={{ display: 'flex', alignItems: 'center', padding: '12px 14px', gap: 10 }}>{children}</div>;
}

function RowLeft({ title, sub }) {
  return (
    <div style={{ flex: 1, minWidth: 0 }}>
      <div style={{ fontSize: 14, fontWeight: 500 }}>{title}</div>
      {sub && <div style={{ fontSize: 11, color: 'rgba(255,255,255,0.5)', marginTop: 1, fontFamily: window.CUMULUS.fonts.mono }}>{sub}</div>}
    </div>
  );
}

function Mono({ children }) {
  return <div style={{ fontSize: 12, fontFamily: window.CUMULUS.fonts.mono, color: 'rgba(255,255,255,0.55)' }}>{children}</div>;
}

function Pill({ children, color }) {
  return (
    <div style={{ fontSize: 10, fontFamily: window.CUMULUS.fonts.mono, fontWeight: 700, letterSpacing: '0.08em', padding: '3px 8px', borderRadius: 6, background: `${color}22`, color, border: `0.5px solid ${color}55` }}>{children}</div>
  );
}

function SourceRow({ icon, name, endpoint, status, onEdit }) {
  const colors = { healthy: '#4ADE80', stale: '#F5A524', error: '#FF6E7A', disabled: 'rgba(255,255,255,0.3)' };
  const label = { healthy: 'OK', stale: 'STALE', error: 'ERROR', disabled: 'OFF' }[status];
  const f = window.CUMULUS.fonts;
  return (
    <div onClick={onEdit} style={{ display: 'flex', alignItems: 'center', padding: '12px 14px', gap: 12, cursor: 'pointer' }}>
      <div style={{ fontSize: 22 }}>{icon}</div>
      <div style={{ flex: 1, minWidth: 0 }}>
        <div style={{ fontSize: 14, fontWeight: 500, marginBottom: 2 }}>{name}</div>
        <div style={{ fontSize: 11, fontFamily: f.mono, color: 'rgba(255,255,255,0.5)', overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>{endpoint}</div>
      </div>
      <Pill color={colors[status]}>{label}</Pill>
      <svg width="7" height="12" viewBox="0 0 7 12"><path d="M1 1 L6 6 L1 11" stroke="rgba(255,255,255,0.3)" strokeWidth="1.5" fill="none" strokeLinecap="round"/></svg>
    </div>
  );
}

function SourceEditor({ sourceKey, source, updSource, close }) {
  const f = window.CUMULUS.fonts;
  return (
    <div style={{ margin: '8px 14px 4px', padding: 14, borderRadius: 16, background: 'rgba(139,124,255,0.08)', border: '1px solid rgba(139,124,255,0.25)' }}>
      <div style={{ display: 'flex', justifyContent: 'space-between', marginBottom: 10 }}>
        <div style={{ fontSize: 11, fontFamily: f.mono, fontWeight: 700, color: '#C7BDFF', letterSpacing: '0.1em' }}>EDIT ENDPOINT</div>
        <div onClick={close} style={{ fontSize: 12, color: 'rgba(255,255,255,0.6)', cursor: 'pointer' }}>Done</div>
      </div>
      <Field label="URL template" value={source.url} mono />
      <Field label="Auth header" value={source.auth || 'Bearer ••••7b2f'} mono />
      <div style={{ display: 'flex', gap: 8, marginTop: 10 }}>
        <div style={{ flex: 1 }}><Field label="Timeout" value="8s" mono /></div>
        <div style={{ flex: 1 }}><Field label="Retries" value="3" mono /></div>
      </div>
      <div style={{ display: 'flex', gap: 8, marginTop: 12 }}>
        <TinyBtn>Test</TinyBtn>
        <TinyBtn>Duplicate</TinyBtn>
        <TinyBtn danger>Disable</TinyBtn>
      </div>
    </div>
  );
}

function Field({ label, value, mono }) {
  const f = window.CUMULUS.fonts;
  return (
    <div style={{ marginBottom: 8 }}>
      <div style={{ fontSize: 9, fontFamily: f.mono, color: 'rgba(255,255,255,0.45)', letterSpacing: '0.1em', fontWeight: 700, marginBottom: 4 }}>{label.toUpperCase()}</div>
      <div style={{ padding: '8px 10px', background: 'rgba(0,0,0,0.3)', borderRadius: 8, fontFamily: mono ? f.mono : f.ui, fontSize: 12, color: '#fff', border: '0.5px solid rgba(255,255,255,0.08)' }}>
        {value}
      </div>
    </div>
  );
}

function TinyBtn({ children, danger }) {
  return <div style={{ padding: '6px 12px', borderRadius: 8, fontSize: 12, fontWeight: 600, background: danger ? 'rgba(255,110,122,0.15)' : 'rgba(255,255,255,0.1)', color: danger ? '#FF6E7A' : '#fff', cursor: 'pointer' }}>{children}</div>;
}

function ContainerRow({ name, image, status, ports }) {
  const f = window.CUMULUS.fonts;
  const dot = { healthy: '#4ADE80', updating: '#F5A524', error: '#FF6E7A' }[status];
  return (
    <div style={{ padding: '10px 14px', display: 'flex', alignItems: 'center', gap: 10 }}>
      <div style={{ width: 7, height: 7, borderRadius: 4, background: dot, flexShrink: 0, boxShadow: `0 0 6px ${dot}88` }} />
      <div style={{ flex: 1, minWidth: 0 }}>
        <div style={{ fontSize: 13, fontFamily: f.mono, fontWeight: 600 }}>{name}</div>
        <div style={{ fontSize: 10, fontFamily: f.mono, color: 'rgba(255,255,255,0.45)', overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>{image}</div>
      </div>
      <div style={{ fontSize: 10, fontFamily: f.mono, color: 'rgba(255,255,255,0.5)' }}>{ports}</div>
    </div>
  );
}

function ToggleRow({ label, sub, value, onChange }) {
  return (
    <div style={{ display: 'flex', alignItems: 'center', padding: '12px 14px', gap: 10 }}>
      <RowLeft title={label} sub={sub} />
      <div onClick={() => onChange(!value)} style={{
        width: 44, height: 26, borderRadius: 13,
        background: value ? '#8B7CFF' : 'rgba(255,255,255,0.15)',
        position: 'relative', cursor: 'pointer', flexShrink: 0,
        transition: 'background 0.2s',
      }}>
        <div style={{ position: 'absolute', top: 2, left: value ? 20 : 2, width: 22, height: 22, borderRadius: 11, background: '#fff', transition: 'left 0.2s', boxShadow: '0 1px 3px rgba(0,0,0,0.3)' }} />
      </div>
    </div>
  );
}

function SelectRow({ label, value, options, onChange }) {
  const [open, setOpen] = React.useState(false);
  return (
    <div style={{ padding: '4px 14px 8px' }}>
      <div style={{ display: 'flex', alignItems: 'center', padding: '8px 0', cursor: 'pointer' }} onClick={() => setOpen(!open)}>
        <div style={{ flex: 1, fontSize: 14 }}>{label}</div>
        <div style={{ fontSize: 13, color: 'rgba(255,255,255,0.5)', fontFamily: window.CUMULUS.fonts.mono, marginRight: 6 }}>{value}</div>
        <svg width="10" height="6" viewBox="0 0 10 6"><path d="M1 1 L5 5 L9 1" stroke="rgba(255,255,255,0.4)" strokeWidth="1.4" fill="none" strokeLinecap="round"/></svg>
      </div>
      {open && (
        <div style={{ display: 'flex', gap: 6, flexWrap: 'wrap', paddingTop: 4 }}>
          {options.map(o => (
            <div key={o} onClick={() => { onChange(o); setOpen(false); }} style={{ padding: '5px 10px', borderRadius: 8, fontSize: 12, cursor: 'pointer', background: value === o ? 'rgba(139,124,255,0.6)' : 'rgba(255,255,255,0.08)', fontFamily: window.CUMULUS.fonts.mono }}>{o}</div>
          ))}
        </div>
      )}
    </div>
  );
}

function BigBtn({ children, primary }) {
  return (
    <div style={{
      flex: 1, padding: '12px 14px', borderRadius: 14, textAlign: 'center', fontSize: 14, fontWeight: 600, cursor: 'pointer',
      background: primary ? '#8B7CFF' : 'rgba(255,255,255,0.08)',
      color: '#fff',
      border: primary ? 'none' : '1px solid rgba(255,255,255,0.1)',
    }}>{children}</div>
  );
}

Object.assign(window, { SettingsScreen });
