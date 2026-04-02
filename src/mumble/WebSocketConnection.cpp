// Copyright The Mumble Developers. All rights reserved.
// Use of this source code is governed by a BSD-style license
// that can be found in the LICENSE file at the root of the
// Mumble source tree or at <https://www.mumble.info/LICENSE>.

#ifdef USE_WEBSOCKET

#	include "WebSocketConnection.h"

#	include "crypto/CryptStateOCB2.h"

#	include <QtCore/QtEndian>
#	include <QtWebSockets/QWebSocket>

#	include <google/protobuf/message.h>

WebSocketConnection::WebSocketConnection(QObject *parent, QWebSocket *socket)
	: QObject(parent), m_socket(socket), m_type(Mumble::Protocol::TCPMessageType::Version), m_packetLength(-1),
	  bDisconnectedEmitted(false) {
	csCrypt = std::make_unique< CryptStateOCB2 >();

	connect(m_socket, &QWebSocket::binaryMessageReceived, this, &WebSocketConnection::onBinaryMessageReceived);
	connect(m_socket, &QWebSocket::errorOccurred, this, &WebSocketConnection::onError);
	connect(m_socket, &QWebSocket::disconnected, this, &WebSocketConnection::onDisconnected);
	connect(m_socket, QOverload< const QList< QSslError > & >::of(&QWebSocket::sslErrors), this,
			&WebSocketConnection::onSslErrors);
}

WebSocketConnection::~WebSocketConnection() {
}

// static
void WebSocketConnection::messageToNetwork(const ::google::protobuf::Message &msg,
										   Mumble::Protocol::TCPMessageType msgType, QByteArray &cache) {
#	if GOOGLE_PROTOBUF_VERSION >= 3004000
	std::size_t len = msg.ByteSizeLong();
#	else
	std::size_t len = msg.ByteSize();
#	endif
	if (len > 0x7fffff)
		return;
	cache.resize(static_cast< int >(len + 6));
	unsigned char *uc = reinterpret_cast< unsigned char * >(cache.data());
	qToBigEndian< quint16 >(static_cast< quint16 >(msgType), &uc[0]);
	qToBigEndian< quint32 >(static_cast< unsigned int >(len), &uc[2]);
	msg.SerializeToArray(uc + 6, static_cast< int >(len));
}

void WebSocketConnection::sendMessage(const ::google::protobuf::Message &msg,
									  Mumble::Protocol::TCPMessageType msgType, QByteArray &cache) {
	if (cache.isEmpty()) {
		messageToNetwork(msg, msgType, cache);
	}
	sendMessage(cache);
}

void WebSocketConnection::sendMessage(const QByteArray &qbaMsg) {
	if (!qbaMsg.isEmpty()) {
		m_socket->sendBinaryMessage(qbaMsg);
	}
}

void WebSocketConnection::disconnectSocket(bool force) {
	if (m_socket->state() == QAbstractSocket::UnconnectedState) {
		emit connectionClosed(QAbstractSocket::UnknownSocketError, QString());
		return;
	}

	if (force) {
		m_socket->abort();
	} else {
		m_socket->close();
	}
}

void WebSocketConnection::forceFlush() {
	m_socket->flush();
}

void WebSocketConnection::proceedAnyway() {
	m_socket->ignoreSslErrors();
}

void WebSocketConnection::onBinaryMessageReceived(const QByteArray &message) {
	// Each WebSocket message carries exactly one Mumble framed packet:
	// 2-byte big-endian message type + 4-byte big-endian payload length + payload.
	m_readBuffer.append(message);

	while (true) {
		if (m_packetLength == -1) {
			if (m_readBuffer.size() < 6) {
				return;
			}
			const unsigned char *hdr =
				reinterpret_cast< const unsigned char * >(m_readBuffer.constData());
			m_type        = static_cast< Mumble::Protocol::TCPMessageType >(qFromBigEndian< quint16 >(hdr));
			m_packetLength = qFromBigEndian< qint32 >(hdr + 2);
		}

		if (m_packetLength > 0x7fffff) {
			qWarning("WebSocketConnection: host tried to send huge packet (%d bytes)", m_packetLength);
			disconnectSocket(true);
			return;
		}

		if (m_readBuffer.size() < 6 + m_packetLength) {
			return;
		}

		QByteArray payload = m_readBuffer.mid(6, m_packetLength);
		m_readBuffer.remove(0, 6 + m_packetLength);
		m_packetLength = -1;

		emit message(m_type, payload);
	}
}

void WebSocketConnection::onError(QAbstractSocket::SocketError error) {
	emit connectionClosed(error, m_socket->errorString());
}

void WebSocketConnection::onDisconnected() {
	emit connectionClosed(QAbstractSocket::UnknownSocketError, QString());
}

void WebSocketConnection::onSslErrors(const QList< QSslError > &errors) {
	emit handleSslErrors(errors);
}

QList< QSslCertificate > WebSocketConnection::peerCertificateChain() const {
	return m_socket->sslConfiguration().peerCertificateChain();
}

QSslCipher WebSocketConnection::sessionCipher() const {
	return m_socket->sslConfiguration().sessionCipher();
}

QHostAddress WebSocketConnection::peerAddress() const {
	return m_socket->peerAddress();
}

quint16 WebSocketConnection::peerPort() const {
	return m_socket->peerPort();
}

QHostAddress WebSocketConnection::localAddress() const {
	return m_socket->localAddress();
}

quint16 WebSocketConnection::localPort() const {
	return m_socket->localPort();
}

#endif // USE_WEBSOCKET
