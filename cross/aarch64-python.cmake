execute_process(
	COMMAND /usr/bin/python3 -c "import sys; print(f'{sys.version_info.major}.{sys.version_info.minor}')"
	OUTPUT_VARIABLE ORPHEUS_PYTHON_VERSION
	OUTPUT_STRIP_TRAILING_WHITESPACE
	COMMAND_ERROR_IS_FATAL ANY
)

set(Python_EXECUTABLE "/usr/bin/python3" CACHE FILEPATH "")
set(Python_INCLUDE_DIR "/usr/include/python${ORPHEUS_PYTHON_VERSION}" CACHE PATH "")
set(ORPHEUS_PYTHON_LIBRARY "/usr/lib/aarch64-linux-gnu/libpython${ORPHEUS_PYTHON_VERSION}.so")

if(NOT EXISTS "${ORPHEUS_PYTHON_LIBRARY}")
	message(FATAL_ERROR "Target Python library not found for aarch64")
endif()

set(Python_LIBRARY "${ORPHEUS_PYTHON_LIBRARY}" CACHE FILEPATH "")
