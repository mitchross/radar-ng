import unittest

from backend.open_meteo_sync.activities import critical_error


class CriticalErrorTest(unittest.TestCase):
    def test_flags_the_silent_linkat_failure(self):
        # openmeteo-api 1.5.6 printed this on stdout and exited 0.
        lines = [
            "[ INFO ] Downloading 2 files (147.4 MB)",
            "[ INFO ] Download https://openmeteo.s3.amazonaws.com/... completed 147.4 MB in 3.4s.",
            "[ CRITICAL ] Error during sync linkAt(error: -1)",
        ]
        self.assertEqual(critical_error(lines), "[ CRITICAL ] Error during sync linkAt(error: -1)")

    def test_errors_count_too(self):
        self.assertTrue(critical_error(["[ ERROR ] Could not download chunk"]))

    def test_a_clean_sync_passes(self):
        lines = [
            "[ INFO ] Checking for files with more than 1 past days data",
            "[ INFO ] ncep_hrrr_conus 16 files completed 1.57 GB in 54s. Average speed 29.6 MB/s",
        ]
        self.assertIsNone(critical_error(lines))


if __name__ == "__main__":
    unittest.main()
