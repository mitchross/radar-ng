import { useState } from "react";
import {
  View,
  Text,
  StyleSheet,
  ScrollView,
  Pressable,
  useWindowDimensions,
} from "react-native";
import { useForecast } from "../../hooks/useForecast";
import { CurrentConditions } from "./CurrentConditions";
import { HourlyScroll } from "./HourlyScroll";
import { DailyForecast } from "./DailyForecast";

type SheetState = "collapsed" | "half" | "full";

export function ForecastSheet() {
  const { data: forecast, isLoading } = useForecast();
  const [state, setState] = useState<SheetState>("collapsed");
  const { height: screenHeight } = useWindowDimensions();

  const heights: Record<SheetState, number> = {
    collapsed: 90,
    half: screenHeight * 0.4,
    full: screenHeight * 0.75,
  };

  const cycleState = () => {
    const order: SheetState[] = ["collapsed", "half", "full"];
    const idx = order.indexOf(state);
    setState(order[(idx + 1) % order.length]);
  };

  if (!forecast && !isLoading) return null;

  return (
    <View style={[styles.container, { height: heights[state] }]}>
      <Pressable onPress={cycleState} style={styles.handleTouchArea}>
        <View style={styles.handleBar} />
        {state === "collapsed" && forecast && (
          <Text style={styles.peekText}>
            {Math.round(forecast.current.temperature_2m)}{"\u00B0"} — Tap for forecast
          </Text>
        )}
      </Pressable>
      {isLoading ? (
        <View style={styles.loading}>
          <Text style={styles.loadingText}>Loading forecast...</Text>
        </View>
      ) : forecast ? (
        <ScrollView
          scrollEnabled={state === "full"}
          showsVerticalScrollIndicator={false}
        >
          {state !== "collapsed" && (
            <>
              <CurrentConditions forecast={forecast} />
              <HourlyScroll forecast={forecast} />
            </>
          )}
          {state === "full" && <DailyForecast forecast={forecast} />}
        </ScrollView>
      ) : null}
    </View>
  );
}

const styles = StyleSheet.create({
  container: {
    backgroundColor: "#1a1a2e",
    borderTopLeftRadius: 16,
    borderTopRightRadius: 16,
    overflow: "hidden",
  },
  handleTouchArea: {
    alignItems: "center",
    paddingVertical: 12,
    minHeight: 44,
    justifyContent: "center",
  },
  handleBar: {
    width: 40,
    height: 4,
    borderRadius: 2,
    backgroundColor: "#555",
  },
  peekText: {
    color: "#aaa",
    fontSize: 13,
    marginTop: 8,
  },
  loading: {
    padding: 20,
    alignItems: "center",
  },
  loadingText: {
    color: "#888",
    fontSize: 14,
  },
});
