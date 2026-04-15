import { useCallback, useMemo, useRef } from "react";
import { View, Text, StyleSheet } from "react-native";
import { ScrollView } from "react-native-gesture-handler";
import BottomSheet from "@gorhom/bottom-sheet";
import { useForecast } from "../../hooks/useForecast";
import { CurrentConditions } from "./CurrentConditions";
import { HourlyScroll } from "./HourlyScroll";
import { DailyForecast } from "./DailyForecast";

export function ForecastSheet() {
  const bottomSheetRef = useRef<BottomSheet>(null);
  const snapPoints = useMemo(() => [80, "35%", "80%"], []);
  const { data: forecast, isLoading } = useForecast();

  const renderHandle = useCallback(
    () => (
      <View style={styles.handle}>
        <View style={styles.handleBar} />
      </View>
    ),
    []
  );

  if (!forecast && !isLoading) return null;

  return (
    <BottomSheet
      ref={bottomSheetRef}
      index={0}
      snapPoints={snapPoints}
      handleComponent={renderHandle}
      backgroundStyle={styles.background}
      enablePanDownToClose={false}
    >
      {isLoading ? (
        <View style={styles.loading}>
          <Text style={styles.loadingText}>Loading forecast...</Text>
        </View>
      ) : forecast ? (
        <ScrollView>
          <CurrentConditions forecast={forecast} />
          <HourlyScroll forecast={forecast} />
          <DailyForecast forecast={forecast} />
        </ScrollView>
      ) : null}
    </BottomSheet>
  );
}

const styles = StyleSheet.create({
  background: {
    backgroundColor: "#1a1a2e",
    borderTopLeftRadius: 16,
    borderTopRightRadius: 16,
  },
  handle: {
    alignItems: "center",
    paddingTop: 10,
    paddingBottom: 4,
  },
  handleBar: {
    width: 40,
    height: 4,
    borderRadius: 2,
    backgroundColor: "#555",
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
