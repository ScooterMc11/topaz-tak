// Global settings panel (editable). Writes through onGlobal.
import { useState } from "react";
import Card from "@mui/material/Card";
import CardContent from "@mui/material/CardContent";
import Collapse from "@mui/material/Collapse";
import Typography from "@mui/material/Typography";
import Box from "@mui/material/Box";
import IconButton from "@mui/material/IconButton";
import Tooltip from "@mui/material/Tooltip";
import ExpandMoreIcon from "@mui/icons-material/ExpandMore";
import ChevronRightIcon from "@mui/icons-material/ChevronRight";
import TextField from "@mui/material/TextField";
import FormControlLabel from "@mui/material/FormControlLabel";
import Switch from "@mui/material/Switch";
import ToggleButton from "@mui/material/ToggleButton";
import ToggleButtonGroup from "@mui/material/ToggleButtonGroup";
import SliderRow from "./SliderRow";
import type { AppConfig, ConfigSchema, GlobalConfig } from "../types";

interface GlobalPanelProps {
  config: AppConfig;
  schema: ConfigSchema;
  onGlobal: (patch: Partial<GlobalConfig>) => void;
}

export default function GlobalPanel({ config, schema, onGlobal }: GlobalPanelProps) {
  const g = config.global;
  const example = g.passwordTemplate.replace("{user}", "AdvancedBot6");
  const [collapsed, setCollapsed] = useState(true);
  // Which stage preset matches the current host+port (null = custom).
  const currentStage =
    schema.stages.find((s) => s.host === g.serverHost && s.port === g.port)?.name ?? null;

  return (
    <Card variant="outlined" sx={{ mb: 3 }}>
      <CardContent sx={{ "&:last-child": { pb: collapsed ? 2 : undefined } }}>
        <Box sx={{ display: "flex", alignItems: "center", gap: 0.5, mb: collapsed ? 0 : 2 }}>
          <Tooltip title={collapsed ? "Expand" : "Collapse"}>
            <IconButton size="small" onClick={() => setCollapsed((c) => !c)}>
              {collapsed ? <ChevronRightIcon fontSize="small" /> : <ExpandMoreIcon fontSize="small" />}
            </IconButton>
          </Tooltip>
          <Typography variant="subtitle1">Global settings</Typography>
        </Box>

        <Collapse in={!collapsed} timeout="auto" unmountOnExit>
        <Box sx={{ mb: 2 }}>
          <Typography variant="caption" color="text.secondary" sx={{ display: "block", mb: 0.5 }}>
            Server target
          </Typography>
          <ToggleButtonGroup
            size="small"
            exclusive
            value={currentStage}
            onChange={(_e, stageName) => {
              if (!stageName) return;
              const s = schema.stages.find((x) => x.name === stageName);
              if (s) onGlobal({ serverHost: s.host, port: s.port });
            }}
          >
            {schema.stages.map((s) => (
              <ToggleButton key={s.name} value={s.name} sx={{ textTransform: "none", px: 2 }}>
                {s.name}
                <Typography variant="caption" color="text.secondary" sx={{ ml: 0.75 }}>
                  {s.host}:{s.port}
                </Typography>
              </ToggleButton>
            ))}
          </ToggleButtonGroup>
          {currentStage === null && (
            <Typography variant="caption" color="warning.main" sx={{ ml: 1 }}>
              custom
            </Typography>
          )}
        </Box>

        <Box sx={{ display: "flex", flexWrap: "wrap", gap: 2, alignItems: "center" }}>
          <TextField
            label="Server host"
            size="small"
            value={g.serverHost}
            onChange={(e) => onGlobal({ serverHost: e.target.value })}
          />
          <TextField
            label="Port"
            size="small"
            type="number"
            value={g.port}
            onChange={(e) => onGlobal({ port: Number(e.target.value) })}
            sx={{ width: 110 }}
          />
          <TextField
            label="Password template"
            size="small"
            value={g.passwordTemplate}
            onChange={(e) => onGlobal({ passwordTemplate: e.target.value })}
            helperText={`{user} → e.g. "${example}"`}
          />
          <FormControlLabel
            control={
              <Switch checked={g.rated} onChange={(e) => onGlobal({ rated: e.target.checked })} />
            }
            label="Rated"
          />
          <FormControlLabel
            control={
              <Switch
                checked={g.linkSizes}
                onChange={(e) => onGlobal({ linkSizes: e.target.checked })}
              />
            }
            label="Link 5x5 / 6x6"
          />
        </Box>
        <Box sx={{ maxWidth: 320, mt: 2 }}>
          <SliderRow
            label="Stagger (seconds between launches)"
            value={g.staggerSeconds}
            range={schema.ranges.staggerSeconds}
            onChange={(v) => onGlobal({ staggerSeconds: v as number })}
          />
        </Box>
        </Collapse>
      </CardContent>
    </Card>
  );
}
