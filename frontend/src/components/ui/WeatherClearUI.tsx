import type { ReactNode } from "react";
import {
  Pressable,
  StyleSheet,
  Text,
  View,
  type ViewProps,
} from "react-native";
import { useWeatherClearTheme } from "../../theme/WeatherClearThemeProvider";

export function WeatherClearCard({
  children,
  style,
  ...props
}: ViewProps & { children: ReactNode }) {
  const { theme } = useWeatherClearTheme();
  return (
    <View
      {...props}
      style={[
        styles.card,
        {
          backgroundColor: theme.colors.surface,
          borderColor: theme.colors.border,
        },
        style,
      ]}
    >
      {children}
    </View>
  );
}

export function SectionLabel({
  children,
  trailing,
}: {
  children: string;
  trailing?: string;
}) {
  const { theme } = useWeatherClearTheme();
  return (
    <View style={styles.sectionLabelRow}>
      <Text
        accessibilityRole="header"
        style={[
          styles.sectionLabel,
          {
            color: theme.colors.textFaint,
            fontFamily: theme.typography.uiBold,
          },
        ]}
      >
        {children}
      </Text>
      {trailing ? (
        <Text
          style={[
            styles.sectionTrailing,
            {
              color: theme.colors.textMuted,
              fontFamily: theme.typography.uiSemibold,
            },
          ]}
        >
          {trailing}
        </Text>
      ) : null}
    </View>
  );
}

export function ScreenHeader({
  kicker,
  title,
  action,
}: {
  kicker?: string;
  title: string;
  action?: ReactNode;
}) {
  const { theme } = useWeatherClearTheme();
  return (
    <View style={styles.screenHeader}>
      <View style={styles.screenHeaderCopy}>
        {kicker ? (
          <Text
            style={[
              styles.kicker,
              {
                color: theme.colors.textFaint,
                fontFamily: theme.typography.uiBold,
              },
            ]}
          >
            {kicker}
          </Text>
        ) : null}
        <Text
          accessibilityRole="header"
          style={[
            styles.screenTitle,
            {
              color: theme.colors.text,
              fontFamily: theme.typography.display,
            },
          ]}
        >
          {title}
        </Text>
      </View>
      {action ?? null}
    </View>
  );
}

export function SegmentedControl<T extends string>({
  accessibilityLabel,
  options,
  value,
  onChange,
}: {
  accessibilityLabel: string;
  options: readonly { label: string; value: T }[];
  value: T;
  onChange: (value: T) => void;
}) {
  const { theme } = useWeatherClearTheme();
  return (
    <View
      accessibilityLabel={accessibilityLabel}
      accessibilityRole="radiogroup"
      style={[
        styles.segmented,
        { backgroundColor: theme.colors.surfaceMuted },
      ]}
    >
      {options.map((option) => {
        const selected = option.value === value;
        return (
          <Pressable
            key={option.value}
            accessibilityRole="radio"
            accessibilityState={{ selected }}
            accessibilityLabel={option.label}
            hitSlop={4}
            onPress={() => onChange(option.value)}
            style={[
              styles.segment,
              selected
                ? {
                    backgroundColor: theme.colors.accent,
                    borderColor: theme.colors.accentBorder,
                  }
                : null,
            ]}
          >
            <Text
              style={[
                styles.segmentText,
                {
                  color: selected ? "#ffffff" : theme.colors.textMuted,
                  fontFamily: theme.typography.uiSemibold,
                },
              ]}
            >
              {option.label}
            </Text>
          </Pressable>
        );
      })}
    </View>
  );
}

export function ScreenState({
  kind,
  title,
  message,
  actionLabel,
  onAction,
}: {
  kind: "loading" | "error" | "empty";
  title: string;
  message: string;
  actionLabel?: string;
  onAction?: () => void;
}) {
  const { theme } = useWeatherClearTheme();
  return (
    <View
      accessibilityRole={kind === "error" ? "alert" : "summary"}
      accessibilityLabel={`${title}. ${message}`}
      style={styles.screenState}
    >
      <View
        style={[
          styles.stateMark,
          {
            backgroundColor:
              kind === "error"
                ? theme.colors.destructive
                : kind === "empty"
                  ? theme.colors.success
                  : theme.colors.accent,
          },
        ]}
      />
      <Text
        selectable
        style={[
          styles.stateTitle,
          {
            color: theme.colors.text,
            fontFamily: theme.typography.display,
          },
        ]}
      >
        {title}
      </Text>
      <Text
        selectable
        style={[
          styles.stateMessage,
          {
            color: theme.colors.textMuted,
            fontFamily: theme.typography.ui,
          },
        ]}
      >
        {message}
      </Text>
      {actionLabel && onAction ? (
        <Pressable
          accessibilityRole="button"
          accessibilityLabel={actionLabel}
          onPress={onAction}
          style={[
            styles.stateAction,
            { backgroundColor: theme.colors.accent },
          ]}
        >
          <Text
            style={[
              styles.stateActionText,
              { fontFamily: theme.typography.uiSemibold },
            ]}
          >
            {actionLabel}
          </Text>
        </Pressable>
      ) : null}
    </View>
  );
}

const styles = StyleSheet.create({
  card: {
    borderWidth: 1,
    borderRadius: 20,
    borderCurve: "continuous",
    overflow: "hidden",
  },
  sectionLabelRow: {
    minHeight: 28,
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "space-between",
    gap: 12,
  },
  sectionLabel: {
    fontSize: 10,
    letterSpacing: 1.5,
  },
  sectionTrailing: {
    fontSize: 11,
  },
  screenHeader: {
    minHeight: 56,
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "space-between",
    gap: 16,
  },
  screenHeaderCopy: {
    flex: 1,
    gap: 3,
  },
  kicker: {
    fontSize: 10,
    letterSpacing: 1.7,
  },
  screenTitle: {
    fontSize: 34,
    lineHeight: 38,
  },
  segmented: {
    flexDirection: "row",
    padding: 3,
    borderRadius: 12,
    borderCurve: "continuous",
    gap: 2,
  },
  segment: {
    minWidth: 44,
    minHeight: 44,
    paddingHorizontal: 10,
    borderRadius: 9,
    borderCurve: "continuous",
    borderWidth: 1,
    borderColor: "transparent",
    alignItems: "center",
    justifyContent: "center",
  },
  segmentText: {
    fontSize: 11,
  },
  screenState: {
    flex: 1,
    minHeight: 360,
    alignItems: "center",
    justifyContent: "center",
    padding: 32,
    gap: 10,
  },
  stateMark: {
    width: 12,
    height: 12,
    borderRadius: 6,
    marginBottom: 8,
  },
  stateTitle: {
    fontSize: 28,
    lineHeight: 32,
    textAlign: "center",
  },
  stateMessage: {
    maxWidth: 320,
    fontSize: 14,
    lineHeight: 21,
    textAlign: "center",
  },
  stateAction: {
    minWidth: 44,
    minHeight: 44,
    marginTop: 8,
    paddingHorizontal: 18,
    borderRadius: 12,
    borderCurve: "continuous",
    alignItems: "center",
    justifyContent: "center",
  },
  stateActionText: {
    color: "#ffffff",
    fontSize: 14,
  },
});
