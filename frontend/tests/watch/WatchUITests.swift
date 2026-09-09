import XCTest

final class WatchUITests: XCTestCase {
    @MainActor
    func testRadarAndForecast() throws {
        continueAfterFailure = false
        let app = XCUIApplication()
        app.launch()
        let system = XCUIApplication(bundleIdentifier: "com.apple.Carousel")
        for _ in 0..<3 {
            if system.buttons["Allow While Using App"].exists {
                system.buttons["Allow While Using App"].tap()
                break
            }
            if system.buttons["Allow Once"].exists {
                system.buttons["Allow Once"].tap()
                break
            }
            if app.buttons["Allow Once"].exists {
                app.buttons["Allow Once"].tap()
                break
            }
            break
        }
        app.activate()
        capture("Watch launch")
        print(app.debugDescription)
        XCTAssertTrue(app.buttons["watch-radar-refresh"].waitForExistence(timeout: 30))
        app.buttons["watch-zoom-out"].tap()
        app.buttons["watch-zoom-in"].tap()
        app.buttons["watch-radar-refresh"].tap()
        capture("Watch radar")
        app.swipeUp()
        XCTAssertTrue(app.staticTexts["Forecast"].waitForExistence(timeout: 10))
        capture("Watch forecast")
        for _ in 0..<4 { app.swipeUp() }
        XCTAssertTrue(app.buttons["watch-forecast-refresh"].exists)
        capture("Watch forecast details")
        app.buttons["watch-forecast-refresh"].tap()
        XCUIDevice.shared.press(.home)
        app.activate()
        XCTAssertTrue(app.buttons["watch-forecast-refresh"].waitForExistence(timeout: 10))
    }

    @MainActor
    func testDeniedLocationUsesNamedFallback() {
        let app = XCUIApplication()
        app.resetAuthorizationStatus(for: .location)
        app.launch()
        let system = XCUIApplication(bundleIdentifier: "com.apple.Carousel")
        let deny = system.buttons.matching(NSPredicate(format: "label == %@ OR label == %@", "Don’t Allow", "Don't Allow")).firstMatch
        XCTAssertTrue(deny.waitForExistence(timeout: 10))
        deny.tap()
        app.activate()
        XCTAssertTrue(app.buttons["watch-radar-refresh"].waitForExistence(timeout: 10))
        app.swipeUp()
        XCTAssertTrue(app.staticTexts["Showing Grand Rapids"].waitForExistence(timeout: 10))
        capture("Watch location denied")
    }

    @MainActor
    func testLaunchPerformance() {
        let app = XCUIApplication()
        let options = XCTMeasureOptions()
        options.iterationCount = 3
        measure(metrics: [XCTApplicationLaunchMetric(waitUntilResponsive: true)], options: options) {
            app.launch()
            app.terminate()
        }
    }

    @MainActor
    private func capture(_ name: String) {
        let attachment = XCTAttachment(screenshot: XCUIScreen.main.screenshot())
        attachment.name = name
        attachment.lifetime = .keepAlways
        add(attachment)
    }
}
