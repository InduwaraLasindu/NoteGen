"""Lightweight native Qt styling; no web engine or animated GPU effects."""

STYLE = """
QWidget { background: #0e1523; color: #e8edf8; font-family: 'Segoe UI'; font-size: 13px; }
QMainWindow, QDialog { background: #0e1523; }
QFrame#panel, QFrame#card { background: #162033; border: 1px solid #26334a; border-radius: 12px; }
QLabel { background: transparent; }
QLabel#brand { font-size: 29px; font-weight: 700; letter-spacing: -1px; }
QLabel#muted { color: #99aac3; }
QLabel#eyebrow { color: #69dccb; font-size: 10px; font-weight: 700; }
QLabel#title { font-size: 24px; font-weight: 600; }
QLabel#metric { font-size: 24px; font-weight: 600; }
QPushButton { background: #202d43; border: 1px solid #31415b; border-radius: 7px; padding: 8px 13px; }
QPushButton:hover { background: #2b3c57; border-color: #5d7da6; }
QPushButton:pressed { background: #334e63; }
QPushButton:disabled { color: #56657b; background: #172131; border-color: #243047; }
QPushButton#primary { background: #68dfcb; color: #092521; border: none; font-weight: 700; }
QPushButton#primary:hover { background: #8debdc; }
QPushButton#primary:disabled { background: #244d4e; color: #6d9391; }
QPushButton#danger { color: #f3b4b4; }
QLineEdit, QPlainTextEdit, QTextEdit, QTextBrowser, QSpinBox, QDoubleSpinBox, QComboBox {
 background: #101a2b; color: #e8edf8; border: 1px solid #2b3b55; border-radius: 6px;
 padding: 7px; selection-background-color: #256f70;
}
QLineEdit:focus, QPlainTextEdit:focus, QComboBox:focus { border-color: #68dfcb; }
QComboBox::drop-down { border: none; width: 22px; }
QComboBox QAbstractItemView { background: #1e2c43; selection-background-color: #315164; }
QCheckBox { background: transparent; spacing: 8px; }
QCheckBox::indicator { width: 16px; height: 16px; border: 1px solid #4c6280; border-radius: 3px; }
QCheckBox::indicator:checked { background: #68dfcb; border-color: #68dfcb; }
QProgressBar { background: #18263a; border: 0; border-radius: 5px; min-height: 10px; color: #aebcd0; text-align: center; }
QProgressBar::chunk { background: #68dfcb; border-radius: 5px; }
QTabWidget::pane { border: 0; }
QTabBar::tab { padding: 11px 16px; background: transparent; color: #99aac3; border-bottom: 2px solid transparent; }
QTabBar::tab:selected { color: #e8edf8; border-bottom: 2px solid #68dfcb; }
QListWidget, QTableWidget, QTreeWidget { background: #101a2b; border: 1px solid #26334a; border-radius: 7px; padding: 4px; }
QListWidget::item { padding: 9px; border-bottom: 1px solid #1d2a40; }
QListWidget::item:selected { background: #254659; color: #e8edf8; border-radius: 5px; }
QHeaderView::section { background: #1f3049; color: #c5d5e9; padding: 7px; border: 0; }
QTableWidget { gridline-color: #344966; }
QScrollArea { border: none; background: transparent; }
QScrollBar:vertical { background: #111a29; width: 9px; }
QScrollBar::handle:vertical { background: #344961; border-radius: 4px; min-height: 30px; }
QScrollBar::add-line:vertical, QScrollBar::sub-line:vertical { height: 0; }
QSplitter::handle { background: #21314a; width: 3px; height: 3px; }
QToolTip { background: #22344d; color: #eef8ff; border: 1px solid #5d7da6; padding: 6px; }
QStatusBar { background: #0b111d; color: #9aadca; }
QMessageBox QLabel { min-width: 320px; }
"""
