import unittest

from draw_drape_images import coherent_pixels


class DrapeMasks(unittest.TestCase):
    def test_preserves_thin_connected_geometry_and_discards_flower_specks(self):
        drawing = {(10, y) for y in range(64)}
        flowers = {(100 + x, 100 + y) for x in range(3) for y in range(2)}
        self.assertEqual(coherent_pixels(drawing | flowers), drawing)
        self.assertEqual(len(drawing), 64)

    def test_keeps_separate_drawing_parts_and_diagonal_edges(self):
        diagonal = {(i, i) for i in range(32)}
        second = {(100, i) for i in range(40)}
        self.assertEqual(coherent_pixels(diagonal | second), diagonal | second)
        self.assertEqual(coherent_pixels(set()), set())


if __name__ == "__main__":
    unittest.main()
