/** Detail sheets for tapped storm cells and warning polygons on the radar map. */
import { Text } from "react-native";
import { MapInfoSheet, PlainEnglish } from "./MapInfoSheet";
import { warningColor, type WarningDetails } from "./WarningsOverlay";
import { compass, hailLabel, isTracked, type StormCellInfo } from "../../lib/stormTracks";
import { useAlertExplanation } from "../../hooks/useAlertExplanation";
import { cumulus } from "../../lib/cumulusTheme";
import { PointOutlooks } from "./PointOutlooks";
import type { PinnedPoint } from "../inspector/Eyedropper";

const dbzColor = (dbz: number) => (dbz >= 60 ? "#d02058" : dbz >= 55 ? "#ff4040" : dbz >= 50 ? "#ff8a00" : "#e0a800");

export function StormCellSheet({ cell, point, onClose }: { cell: StormCellInfo | null; point?: PinnedPoint; onClose: () => void }) {
  if (!cell) return null;
  const tracked = isTracked(cell);
  return (
    <MapInfoSheet
      visible
      eyebrow="STORM CELL"
      title={`${Math.round(cell.peakDbz)} dBZ peak`}
      subtitle={tracked ? `Moving ${compass(cell.bearing)} at ${Math.round(cell.speedMph)} mph` : "Motion not yet tracked"}
      accent={dbzColor(cell.peakDbz)}
      pill={hailLabel(cell.peakDbz)}
      metrics={[
        { label: "AREA", value: Math.round(cell.areaKm2).toLocaleString(), unit: "km²" },
        { label: "SPEED", value: tracked ? `${Math.round(cell.speedMph)}` : "—", unit: "mph" },
      ]}
      onClose={onClose}
    >
      {point ? <PointOutlooks {...point} /> : null}
    </MapInfoSheet>
  );
}

export function WarningSheet({ warning, point, onClose }: { warning: WarningDetails | null; point?: PinnedPoint; onClose: () => void }) {
  const historical = warning?.historical;
  // A replayed warning has no live explanation or point outlook.
  const { explanation, loading } = useAlertExplanation(historical ? null : warning?.id);
  if (!warning) return null;
  if (historical) {
    return (
      <MapInfoSheet
        visible
        eyebrow="HISTORICAL WARNING"
        title={historical.title}
        subtitle={historical.badge || undefined}
        accent={warningColor(warning.event)}
        onClose={onClose}
      >
        {historical.lines.map((line) => (
          <Text key={line} style={{ marginTop: 8, color: cumulus.inkDim, fontSize: 13 }}>{line}</Text>
        ))}
      </MapInfoSheet>
    );
  }
  const until = warning.ends ? new Date(warning.ends).toLocaleString([], { weekday: "short", hour: "numeric", minute: "2-digit" }) : null;
  return (
    <MapInfoSheet
      visible
      eyebrow={warning.event.toUpperCase()}
      title={(warning.areaDesc ?? "").split(";").slice(0, 3).join(", ") || warning.event}
      subtitle={until ? `Until ${until}` : warning.severity}
      accent={warningColor(warning.event)}
      onClose={onClose}
    >
      {point ? <PointOutlooks {...point} /> : null}
      {explanation ? <PlainEnglish what={explanation.what} doText={explanation.do} /> : null}
      {!explanation && loading ? <Text style={{ marginTop: 12, color: cumulus.inkMuted, fontSize: 12 }}>{"✦"} Explaining…</Text> : null}
    </MapInfoSheet>
  );
}
