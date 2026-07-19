// A labeled slider row with a live value readout. Read-only in Phase 2 (disabled); Phase 3 wires
// onChange. Supports a single value or an array (the 3-thumb gap slider).
import Box from "@mui/material/Box";
import Slider from "@mui/material/Slider";
import Typography from "@mui/material/Typography";
import type { NumRange } from "../types";

interface SliderRowProps {
  label: string;
  value: number | number[];
  range: NumRange;
  disabled?: boolean;
  disableSwap?: boolean; // for the multi-thumb gap slider: thumbs stop instead of crossing
  valueText?: string; // overrides the default readout (e.g. "15 / 30 / 45")
  helper?: string;
  onChange?: (value: number | number[]) => void;
}

export default function SliderRow({
  label,
  value,
  range,
  disabled,
  disableSwap,
  valueText,
  helper,
  onChange,
}: SliderRowProps) {
  const readout = valueText ?? (Array.isArray(value) ? value.join(" / ") : String(value));
  return (
    <Box sx={{ mb: 1.5 }}>
      <Box sx={{ display: "flex", justifyContent: "space-between", alignItems: "baseline" }}>
        <Typography variant="body2" color="text.secondary">
          {label}
        </Typography>
        <Typography variant="body2" sx={{ fontVariantNumeric: "tabular-nums" }}>
          {readout}
        </Typography>
      </Box>
      <Slider
        size="small"
        value={value}
        min={range.min}
        max={range.max}
        step={range.step}
        disabled={disabled}
        disableSwap={disableSwap}
        onChange={(_e, v) => onChange?.(v)}
        sx={{ mt: 0.5 }}
      />
      {helper && (
        <Typography variant="caption" color="text.secondary">
          {helper}
        </Typography>
      )}
    </Box>
  );
}
