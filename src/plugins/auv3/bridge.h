// Apple AUv3 host ABI (macOS and iOS). All calls except render, MIDI, parameter scheduling and
// transport run on the app thread, under the existing plugin instance mutex.
#pragma once
#include <stdint.h>
#include <stddef.h>
char *hd_au_discover(void);
void hd_au_free(char *);
void *hd_au_create(uint32_t type, uint32_t subtype, uint32_t manufacturer);
void hd_au_destroy(void *);
// 0 loading, 1 ready, -1 failed. Configuration is performed here, never on audio.
int hd_au_poll(void *, uint32_t rate);
char *hd_au_error(void *);
char *hd_au_parameters(void *);
float hd_au_value(void *, uint32_t index);
float hd_au_take_value(void *, uint32_t index); // NaN when no editor change is pending.
void hd_au_parameter(void *, uint32_t index, float normalized, uint32_t offset);
void hd_au_set_value(void *, uint32_t index, float normalized);
void hd_au_midi(void *, uint32_t offset, const uint8_t *, uint32_t length);
int hd_au_render(void *, float *, float *, uint32_t frames, int effect);
void hd_au_reset(void *);
void hd_au_transport(void *, double tempo, double beat, double bar, double frame,
                     uint32_t numerator, uint32_t denominator, int playing);
uint8_t *hd_au_state(void *, size_t *length); // malloc-owned binary property list.
int hd_au_load_state(void *, const uint8_t *, size_t);
uint32_t hd_au_latency(void *);
void hd_au_offline(void *, int);
int hd_au_has_editor(void *);
int hd_au_open_editor(void *);
void hd_au_close_editor(void *);
// 0 closed, 1 asynchronous request pending, 2 native window visible.
int hd_au_editor_open(void *);
