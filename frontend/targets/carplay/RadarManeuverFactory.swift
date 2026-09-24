import CarPlay
import MapKit

/// Shared by the CarPlay controller and the entitlement-review evidence host.
/// The review UI reads these actual CPManeuver objects; it does not invent turns.
@MainActor
enum RadarManeuverFactory {
    static func makeManeuvers(for driving: RadarDrivingSession) -> [CPManeuver] {
        driving.steps.enumerated().map { index, step in
            let maneuver = CPManeuver()
            maneuver.instructionVariants = [step.instructions]
            maneuver.dashboardInstructionVariants = maneuver.instructionVariants
            maneuver.notificationInstructionVariants = maneuver.instructionVariants
            let symbol = UIImage(systemName: symbol(at: index, steps: driving.steps))
            maneuver.symbolImage = symbol
            maneuver.dashboardSymbolImage = symbol
            maneuver.notificationSymbolImage = symbol
            // Estimates describe the leg BEFORE this maneuver, not the road
            // following it. Live estimates below decrease as GPS progresses.
            let prior = index > 0 ? driving.stepOffsets[index - 1] : 0
            let distance = max(0, driving.stepOffsets[index] - prior)
            maneuver.initialTravelEstimates = estimates(distance: distance, driving: driving)
            return maneuver
        }
    }

    static func currentEstimates(for driving: RadarDrivingSession) -> CPTravelEstimates {
        estimates(distance: driving.maneuverDistance, driving: driving)
    }

    static func tripEstimates(for driving: RadarDrivingSession) -> CPTravelEstimates {
        CPTravelEstimates(distanceRemaining: Measurement(value: driving.remainingDistance, unit: UnitLength.meters),
                          timeRemaining: driving.remainingTime)
    }

    private static func estimates(distance: Double, driving: RadarDrivingSession) -> CPTravelEstimates {
        let remaining = driving.remainingDistance
        let seconds = remaining > 0 ? driving.remainingTime * distance / remaining : 0
        return CPTravelEstimates(distanceRemaining: Measurement(value: distance, unit: UnitLength.meters),
                                timeRemaining: max(0, seconds))
    }

    private static func symbol(at index: Int, steps: [MKRoute.Step]) -> String {
        if let roundabout = RadarRouteProgress.roundaboutSymbol(instruction: steps[index].instructions) {
            return roundabout
        }
        return RadarRouteProgress.turnSymbol(incoming: steps[index].polyline,
            outgoing: index + 1 < steps.count ? steps[index + 1].polyline : nil)
    }
}
