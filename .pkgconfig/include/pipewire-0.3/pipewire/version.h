/* Generated version header for PipeWire */
#ifndef PIPEWIRE_VERSION_H
#define PIPEWIRE_VERSION_H

#define PIPEWIRE_VERSION_MAJOR 1
#define PIPEWIRE_VERSION_MINOR 2
#define PIPEWIRE_VERSION_MICRO 0
#define PIPEWIRE_VERSION_NANO 0

#define PIPEWIRE_API_VERSION "0.3"

#define pw_get_headers_version() ("1.2.0")

#define PW_CHECK_VERSION(major,minor,micro) \
    ((PIPEWIRE_VERSION_MAJOR > (major)) || \
     (PIPEWIRE_VERSION_MAJOR == (major) && PIPEWIRE_VERSION_MINOR > (minor)) || \
     (PIPEWIRE_VERSION_MAJOR == (major) && PIPEWIRE_VERSION_MINOR == (minor) && \
      PIPEWIRE_VERSION_MICRO >= (micro)))

#endif /* PIPEWIRE_VERSION_H */

